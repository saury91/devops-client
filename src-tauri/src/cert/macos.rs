//! macOS backend: the user's login keychain, through Security.framework.
//!
//! Why the classic login keychain instead of the newer data protection keychain: Safari and
//! Chromium read client identities through the legacy `SecIdentity` APIs, and those APIs do not
//! see data protection keychain items. A certificate kept there would simply never be presented,
//! so every request would look certificate-less and the server would fall back to the legacy
//! checks while the client believed it was certificate-bound. `Location::DefaultFileKeychain`
//! puts the identity where the browsers actually look.
//!
//! One-time prompt: macOS scopes a new private key to the application that created it, so the
//! first browser use shows a keychain dialog asking for the login password, with an
//! "Always Allow" button that silences it permanently. That is the platform's standard behaviour
//! for locally installed client certificates; a prompt-free rollout needs an MDM profile, which
//! is a deployment decision rather than a client one. Everything else on macOS 10.15+ — our
//! floor — works without any prompt.
//!
//! Trust settings are the other half of an install, and leaving them out is silent: the
//! certificate here is self-signed, and macOS treats a self-signed certificate as untrusted until
//! it is told otherwise. A browser filters the key store by trust before it offers an identity, so
//! an untrusted certificate is present, is listed by `security find-certificate`, and is still
//! never presented at a handshake. Measured on this machine — a freshly installed certificate
//! reports `CSSMERR_TP_NOT_TRUSTED` and is missing from `security find-identity -v -p ssl-client`,
//! which is precisely the state where every request reaches the server certificate-less while the
//! client believes it is bound. [`MacKeychainProvider::ensure`] therefore grants trust as part of
//! installing.
//!
//! Installing is not the only way a certificate reaches that state. A client that predates this
//! trust handling left an untrusted certificate in the keychain and registered itself with the
//! server anyway, so an upgrade inherits a certificate that `ensure` would once have found and
//! returned untouched. [`MacKeychainProvider::ensure`] therefore treats trust as part of the desired
//! end state rather than a side effect of a fresh install: when the certificate is already present
//! it reads the existing trust settings and repairs them only when they are missing, which keeps
//! repeat calls idempotent and leaves a user's explicit "deny" decision alone.
//!
//! Repairing on install alone would still miss the machine that needs it most. An upgraded terminal
//! that keeps its session never passes through `ensure` again — not on a silent `auto_login`, and
//! not in the heartbeat — so it would hold an unusable certificate indefinitely while the browser
//! sessions it opens were rejected. [`MacKeychainProvider::repair_installed`] is that same trust
//! half on its own, for the periodic path and for the moment before a browser session is handed the
//! certificate; it never installs, so it cannot leave a device holding an identity the server has
//! not been told about.
//!
//! That grant goes to the *user* trust domain. The admin domain raises an authorization dialog and
//! the system domain belongs to Apple, while user settings need neither an administrator nor a
//! configuration profile — which is what keeps an install usable by the signed-in user alone. The
//! cost is that trust is per account rather than per machine, which is the right trade for a
//! client that installs its own certificate anyway.
//!
//! That dialog is also why no trusted-application list is written into the key's ACL here. A
//! keychain ACL names its trusted applications by code signature (cdhash), and every browser
//! update changes that hash: a list pinned at install time would begin denying the very browsers
//! it was meant to admit, and would need rewriting on a schedule this client cannot observe.
//! "Always Allow" instead appends the browser to the ACL at the moment the user grants it, which
//! survives updates. The bindings used here expose no ACL setter anyway — `SecAccessCreate` and
//! `SecKeychainItemSetAccess` are absent from `security-framework-sys` — so widening the list
//! would mean hand-rolled FFI for a result strictly worse than the built-in prompt.
//!
//! Firefox reaches this certificate through `security.osclientcerts.autoload` rather than through
//! an import into its own NSS database, which is what keeps the key non-exportable here too:
//! importing would require handing Firefox a copyable private key. [`super::firefox`] writes that
//! preference; the identity itself stays in the keychain where this module put it.

use super::{
    build_self_signed, cert_info_from_der, parse_cert_meta, sha256_hex, CertCapability, CertError,
    CertInfo, DeviceCertProvider, CERT_LABEL,
};
use rcgen::{RemoteKeyPair, SignatureAlgorithm, PKCS_ECDSA_P256_SHA256};
use security_framework::base::Error;
use security_framework::certificate::SecCertificate;
use security_framework::item::{
    ItemClass, ItemSearchOptions, Limit, Location, Reference, SearchResult,
};
use security_framework::key::{Algorithm, GenerateKeyOptions, KeyType, SecKey};
use security_framework::trust_settings::{Domain, TrustSettings, TrustSettingsForCertificate};
use security_framework_sys::base::errSecItemNotFound;

/// Store name reported to the UI and written to logs.
const STORE: &str = "login keychain";

/// Key store provider backed by the login keychain of the current user.
pub struct MacKeychainProvider;

impl DeviceCertProvider for MacKeychainProvider {
    fn store_name(&self) -> &'static str {
        STORE
    }

    /// Always `full`: 10.15 already exposes every API used here, so any failure left is a
    /// keychain lock or permission problem, which [`MacKeychainProvider::ensure`] surfaces as an
    /// error instead of pretending the machine is limited.
    fn capability(&self) -> CertCapability {
        CertCapability::Full
    }

    fn ensure(&self, owner: &str) -> Result<CertInfo, CertError> {
        // A certificate already in the key store is not returned as-is: being present says nothing
        // about being trusted, and both states that need repair arrive with a certificate already
        // installed — a machine upgrading from a build that never granted trust, and one whose
        // trust entry was reset afterwards. Measured on this machine: with the certificate in the
        // keychain and no user-domain trust entry, `security find-certificate` still lists it,
        // `security find-identity -p ssl-client` marks it `CSSMERR_TP_NOT_TRUSTED`, and the `-v`
        // list omits it — precisely the state where every request reaches the server without a
        // certificate. Trust is therefore settled on this path too, and written only when missing.
        if let Some(certificate) = installed_certificate()? {
            let capability = self.capability_with_trust(&certificate);
            return cert_info_from_der(&certificate.to_der(), capability, STORE).ok_or_else(|| {
                CertError::Platform("read installed certificate metadata".to_string())
            });
        }

        self.issue_certificate(owner)
    }

    /// Installs a replacement while whatever is already in the key store stays usable.
    ///
    /// 与 `ensure` 的唯一区别是不先删旧的：签名、信任授予、返回的元数据都与 `ensure` 在空钥匙串上
    /// 做的完全相同，所以续期成功后的状态与"全新安装"一致；差别只在装新证书失败时 —— 原来那张
    /// 还在，设备仍然能过服务端的指纹校验，而不是变成一台没有任何身份的机器。
    fn ensure_fresh(&self, owner: &str) -> Result<CertInfo, CertError> {
        self.issue_certificate(owner)
    }

    /// Drops every certificate of ours except `keep_fingerprint`.
    ///
    /// 只删证书，不删私钥。要安全地删掉被取代的那把密钥，必须先确认它与保留的那张证书配对，
    /// 而这里没有可靠的办法建立这种配对（`security-framework` 不暴露能比对二者的接口），猜错的
    /// 代价是删掉设备正在用的身份。没有证书的私钥本身认证不了任何东西 —— 浏览器要的是"证书+私钥"
    /// 这一对 —— 残留条目留给下一次 `remove`（登出/重注册）一并清掉。
    ///
    /// @param keep_fingerprint 要保留的那张证书的 SHA-256，其余同名证书一律删除
    /// @return `Err` 表示钥匙串读不了或有条目拒绝删除，此时可能有证书没删掉；
    ///         调用方把它当成"降级状态"而不是失败
    fn keep_only(&self, keep_fingerprint: &str) -> Result<(), CertError> {
        let mut failure: Option<CertError> = None;
        for certificate in search_certificates()? {
            if sha256_hex(&certificate.to_der()) == keep_fingerprint {
                continue;
            }
            if let Err(e) = certificate.delete() {
                failure.get_or_insert(CertError::Platform(format!(
                    "delete superseded certificate: {}",
                    e
                )));
            }
        }
        failure.map_or(Ok(()), Err)
    }

    fn status(&self) -> Option<CertInfo> {
        // A keychain that cannot be searched — locked, or slow enough that Security.framework gives
        // up — reads as "nothing installed". `status` is documented never to fail, and the
        // heartbeat turns the absence into a fresh install attempt on a later cycle, so folding the
        // error away here is intentional and confined to this read-only path.
        let certificate = installed_certificate().ok()??;
        // The capability comes from the certificate's actual trust state, not from what the
        // platform is theoretically capable of: reporting `Full` for a certificate no browser will
        // present is how an untrusted machine passes itself off as working, and it is the state a
        // terminal lands in after upgrading from a build that predates trust handling.
        cert_info_from_der(
            &certificate.to_der(),
            self.reported_capability(&certificate),
            STORE,
        )
    }

    /// Settles trust without installing anything, for the paths that must not create a certificate.
    ///
    /// Reuses [`MacKeychainProvider::capability_with_trust`], the same decision `ensure` makes, so
    /// the two cannot drift on when a repair happens or when a refusal is respected. The returned
    /// capability is discarded deliberately: callers read it back through
    /// [`DeviceCertProvider::status`] when they need it, and the write is the point of this path.
    fn repair_installed(&self) -> Result<(), CertError> {
        if let Some(certificate) = installed_certificate()? {
            let _ = self.capability_with_trust(&certificate);
        }
        Ok(())
    }

    fn remove(&self) -> Result<(), CertError> {
        // Both halves must go: leaving the key behind would let a later `ensure` reinstall a
        // certificate whose fingerprint the server has already revoked, and the device would look
        // bound while actually being rejected.
        //
        // Two traps are avoided here, both of which report success while an identity survives.
        // First, a failed search must not be folded into "nothing there": a search that times out
        // would skip the deletion entirely. Second, deleting only the *first* match is not enough —
        // a machine that ran an older build, or a removal that was interrupted, can hold more than
        // one leftover key under this label, and stopping early leaves a usable private key behind
        // for a later `ensure` to reuse as a stale identity. So every matching item is removed, and
        // one failure does not stop the rest from being cleared.
        let mut failure: Option<CertError> = None;

        for certificate in search_certificates()? {
            if let Err(e) = certificate.delete() {
                failure.get_or_insert(CertError::Platform(format!("delete certificate: {}", e)));
            }
        }
        for key in search_keys()? {
            if let Err(e) = key.delete() {
                failure.get_or_insert(CertError::Platform(format!("delete private key: {}", e)));
            }
        }

        // The aggregated error is the first one: the caller only needs to know that the key store
        // did not end up clean, and a per-item log line would be noise for a single user action.
        failure.map_or(Ok(()), Err)
    }
}

impl MacKeychainProvider {
    /// Capability of `certificate` as it currently stands, without changing anything.
    ///
    /// Read-only, so that [`DeviceCertProvider::status`] can answer truthfully: a certificate that
    /// is installed but untrusted is `Partial`, not `Full`.
    fn reported_capability(&self, certificate: &SecCertificate) -> CertCapability {
        match trust_grant(certificate) {
            TrustGrant::Granted => self.capability(),
            TrustGrant::Denied | TrustGrant::Missing => CertCapability::Partial,
        }
    }

    /// Capability of `certificate`, granting trust first when it is missing.
    ///
    /// The write is conditional on trust actually being absent, which keeps a repeat `ensure` on an
    /// already trusted machine free of key-store writes — the property the idempotency check
    /// depends on. Writing unconditionally would also be safe, since the same grant applied twice
    /// leaves a single entry (measured), but it would put a key-store write on every sign-in.
    ///
    /// A failure degrades the reported capability instead of failing the caller: the certificate is
    /// in the key store and the system TLS stack can still present it, so the machine is not
    /// `unavailable`. An error would also be the wrong shape here — it is reported as "the install
    /// failed" to a caller that cannot tell a trust problem from a key-store problem, and the
    /// renewal path would take it as a reason to try again.
    fn capability_with_trust(&self, certificate: &SecCertificate) -> CertCapability {
        match trust_grant(certificate) {
            TrustGrant::Granted => self.capability(),
            // An explicit refusal is left alone. Overwriting it would silently reverse a decision
            // somebody made about this certificate, and the machine is not broken: the certificate
            // is present and a browser simply will not offer it.
            TrustGrant::Denied => {
                crate::config::log_error(
                    "cert",
                    "certificate trust was explicitly refused; leaving that decision in place",
                );
                CertCapability::Partial
            }
            TrustGrant::Missing => match trust_certificate(certificate) {
                Ok(()) => self.capability(),
                Err(e) => {
                    crate::config::log_error(
                        "cert",
                        &format!("certificate installed but not trusted: {}", e),
                    );
                    CertCapability::Partial
                }
            },
        }
    }

    /// Issues and installs a certificate against a brand-new non-exportable key.
    ///
    /// Shared by the two install paths — [`DeviceCertProvider::ensure`] on an empty key store and
    /// the staged [`DeviceCertProvider::ensure_fresh`] — so the certificate they produce cannot
    /// drift apart, and so the renewal path inherits every decision made for the first install.
    ///
    /// Trust is granted as part of installing rather than left to the user: a self-signed
    /// certificate nothing trusts is never offered by a browser, so an install that stopped at the
    /// keychain would leave a device registered with the server that still cannot authenticate. The
    /// measurement behind this is in the module header.
    ///
    /// @param owner 证书 subject 里的使用者标识（邮箱），服务端据此把证书归属到账号
    /// @return 新证书的指纹/序列号/到期时间与安装后的实际能力
    fn issue_certificate(&self, owner: &str) -> Result<CertInfo, CertError> {
        // The key is generated by the key store and never leaves it; we only get a handle.
        let key = create_persistent_key()?;
        let public = key
            .public_key()
            .and_then(|public| public.external_representation())
            .map(|data| data.to_vec())
            .ok_or_else(|| {
                CertError::Platform("keychain key has no readable public portion".to_string())
            })?;

        let issued = build_self_signed(Box::new(KeychainRemoteKey { key, public }), owner)?;
        let certificate = install_certificate(&issued.der)?;
        let capability = self.capability_with_trust(&certificate);

        Ok(CertInfo {
            fingerprint: issued.fingerprint,
            serial: issued.serial,
            not_after: issued.not_after,
            capability,
            store: STORE,
        })
    }
}

/// What the user trust domain currently says about `certificate`.
#[derive(Debug, PartialEq, Eq)]
enum TrustGrant {
    /// An entry is present and the certificate is trusted.
    Granted,
    /// An entry is present that explicitly refuses trust.
    Denied,
    /// The domain holds no entry for this certificate at all.
    Missing,
}

/// Reads the user-domain trust state of `certificate`.
///
/// The question is not "is there a setting?" but "which of the observed read results is this one?".
/// Measured on this machine the answers are distinguishable, and they map as follows:
///
/// - `Err(errSecItemNotFound)` — no entry for this certificate in the domain. This is the untrusted
///   case, and the one an upgrade from a trust-less build lands in: that certificate reads as
///   `CSSMERR_TP_NOT_TRUSTED` and is absent from `security find-identity -v -p ssl-client`.
/// - `Ok(None)` — an entry exists but carries no TLS-specific setting. That is the shape
///   [`trust_certificate`] writes, since it stores a null settings array; measured as trusted, with
///   the identity present in the valid-only list.
/// - `Ok(Some(TrustRoot | TrustAsRoot))` — explicitly trusted.
/// - `Ok(Some(Deny))` — explicitly refused. Kept separate from a missing grant rather than folded
///   into "not trusted", because the repair path has to tell a refusal apart from a gap.
///
/// Only the user domain is consulted, the same domain the grant is written to.
fn trust_grant(certificate: &SecCertificate) -> TrustGrant {
    trust_grant_from(
        TrustSettings::new(Domain::User).tls_trust_settings_for_certificate(certificate),
    )
}

/// Maps one read result onto [`TrustGrant`].
///
/// Split out from the read so the mapping is unit-testable without a keychain, because it is the
/// load-bearing decision here: folding `Deny` into "no grant" would make
/// [`MacKeychainProvider::capability_with_trust`] overwrite somebody's explicit refusal.
fn trust_grant_from(read: Result<Option<TrustSettingsForCertificate>, Error>) -> TrustGrant {
    match read {
        Ok(Some(TrustSettingsForCertificate::TrustRoot))
        | Ok(Some(TrustSettingsForCertificate::TrustAsRoot))
        | Ok(None) => TrustGrant::Granted,
        Ok(Some(TrustSettingsForCertificate::Deny)) => TrustGrant::Denied,
        // `Unspecified` and `Invalid` are unreachable through this crate, which skips them, and a
        // read error means the domain cannot vouch for the certificate. Both count as no grant, so
        // that an unreadable state is repaired rather than reported as working.
        Ok(Some(TrustSettingsForCertificate::Unspecified))
        | Ok(Some(TrustSettingsForCertificate::Invalid))
        | Err(_) => TrustGrant::Missing,
    }
}

/// The certificate this client is bound to, when the key store holds one.
///
/// Newest wins. A staged renewal holds two of ours between installing the replacement and dropping
/// the superseded one, and a key store that refuses that removal leaves both behind for good.
/// Reporting the newest keeps this client on the certificate the server was told about most
/// recently; picking an arbitrary one would let the two disagree permanently, and the fingerprint
/// mismatch that follows reads to the server as a stolen device rather than a stale key-store entry.
///
/// @return `Ok(None)` 表示钥匙串里没有本客户端的证书；`Err` 表示读取本身失败，
///         准备据此动手（删除/修复）的调用方不能把 `Err` 当成"没有证书"
fn installed_certificate() -> Result<Option<SecCertificate>, CertError> {
    Ok(search_certificates()?
        .into_iter()
        .max_by_key(certificate_expiry))
}

/// Not-after of a key-store certificate as a comparable instant, for [`installed_certificate`].
///
/// `None` sorts before every instant, so a certificate whose validity cannot be parsed — corrupt, or
/// not really a certificate — never displaces a readable one.
fn certificate_expiry(certificate: &SecCertificate) -> Option<i64> {
    let (_, not_after) = parse_cert_meta(&certificate.to_der())?;
    time::OffsetDateTime::parse(&not_after, &time::format_description::well_known::Rfc3339)
        .ok()
        .map(|instant| instant.unix_timestamp())
}

/// Creates the non-exportable P-256 key in the login keychain.
///
/// `Location::DefaultFileKeychain` is what makes the key persistent *and* browser visible; without
/// a location Security.framework would hand back an ephemeral key that disappears with the process.
fn create_persistent_key() -> Result<SecKey, CertError> {
    let mut options = GenerateKeyOptions::default();
    options
        .set_key_type(KeyType::ec())
        .set_size_in_bits(256)
        .set_label(CERT_LABEL)
        .set_location(Location::DefaultFileKeychain);
    SecKey::new(&options).map_err(|e| CertError::Platform(format!("create keychain key: {}", e)))
}

/// Adds the certificate to the login keychain, which is what forms the `SecIdentity` browsers
/// match against the private key created above.
///
/// The certificate is handed back rather than dropped because the step that follows — granting it
/// trust — needs the same `SecCertificate` the key store just accepted, not a fresh parse of the
/// DER that would describe an equivalent-but-separate object.
///
/// @return the certificate as the key store now holds it
fn install_certificate(der: &[u8]) -> Result<SecCertificate, CertError> {
    let certificate = SecCertificate::from_der(der)
        .map_err(|e| CertError::Platform(format!("parse issued certificate: {}", e)))?;
    certificate
        .add_to_keychain(None)
        .map_err(|e| CertError::Platform(format!("add certificate to keychain: {}", e)))?;
    Ok(certificate)
}

/// Marks `certificate` as trusted for the current user, which is what lets a browser offer it.
///
/// The user domain is deliberate. Admin trust settings raise an authorization dialog, and this
/// client installs a certificate for the user who is signing in rather than for the machine, so
/// asking for an administrator password would block the very step that makes the install work.
///
/// The grant covers all uses rather than the SSL policy alone, which is a consequence of the
/// binding: `set_trust_settings_always` passes a null settings array, and the crate exposes no way
/// to write a policy-restricted one — that would mean building the settings array by hand against
/// `SecTrustSettingsSetTrustSettings`. It is acceptable here because the certificate is a non-CA
/// leaf with no extended key usage, so a broader trust entry still vouches for nothing beyond this
/// one certificate, and the matching private key is non-exportable and held in the login keychain.
///
/// There is no inverse of this call here, and that is not a leak either: `security-framework-sys`
/// binds `SecTrustSettingsSetTrustSettings` but not `SecTrustSettingsRemoveTrustSettings`.
///
/// Deleting a certificate does leave its trust entry behind, which a first reading of
/// `security dump-trust-settings` gets wrong: that command lists only the entries it can still match
/// to a certificate in the key store, so a removed certificate's entry vanishes from its output.
/// `security trust-settings-export` shows the same state as `trustList`, and there the entries of
/// earlier certificates are still present. The residue is harmless — an entry is keyed by
/// certificate hash and can never match a later certificate — but it does accumulate one entry per
/// renewal, and clearing it would need FFI the crate does not bind.
///
/// @return `Err` when the domain refused the write, for example in a session with no GUI, where
///         macOS answers `errSecInternalComponent` for per-user trust settings
fn trust_certificate(certificate: &SecCertificate) -> Result<(), CertError> {
    TrustSettings::new(Domain::User)
        .set_trust_settings_always(certificate)
        .map_err(|e| CertError::Platform(format!("set user trust settings: {}", e)))
}

/// Runs an item search, mapping "nothing matched" onto an empty result.
///
/// Security.framework reports an empty result set as `errSecItemNotFound` rather than as zero
/// results, so without this mapping the ordinary states — nothing installed yet, or a removal that
/// already succeeded — would look like a failure. Every other error still surfaces, because a
/// keychain that could not be searched must never be read as "nothing there".
///
/// @param options fully populated search options, borrowed so the caller keeps ownership
/// @param described item kind named in the error message, for example "certificate"
/// @return all matches, an empty vector when none match, or the keychain's own error
fn search_items(
    options: &ItemSearchOptions,
    described: &str,
) -> Result<Vec<SearchResult>, CertError> {
    match options.search() {
        Ok(results) => Ok(results),
        Err(e) if e.code() == errSecItemNotFound => Ok(Vec::new()),
        Err(e) => Err(CertError::Platform(format!(
            "search keychain for {}: {}",
            described, e
        ))),
    }
}

/// Finds every certificate of ours in the login keychain, matched by subject.
///
/// Search is by class only and then filtered in Rust: `kSecAttrLabel` on a certificate added
/// through the legacy API is derived from the subject rather than settable, and the OU carries the
/// machine name, so matching on `kSecAttrSubject` would miss certificates issued by older builds.
///
/// All matches are returned rather than the first, because a removal that stops at one item would
/// report success while a duplicate — left by an older build or an interrupted run — survived.
///
/// @return `Ok(empty)` when the keychain holds no matching certificate; `Err` when the search
///         itself failed, which a caller about to delete must not read as "nothing there"
fn search_certificates() -> Result<Vec<SecCertificate>, CertError> {
    let mut options = ItemSearchOptions::new();
    options
        .class(ItemClass::certificate())
        .limit(Limit::All)
        .load_refs(true)
        .load_data(false);

    let mut found = Vec::new();
    for result in search_items(&options, "certificate")? {
        if let SearchResult::Ref(Reference::Certificate(certificate)) = result {
            if certificate.subject_summary().contains(CERT_LABEL) {
                found.push(certificate);
            }
        }
    }
    Ok(found)
}

/// Finds every private key created for this client, matched by the label set at creation time.
///
/// The label is unique to this application, so every match is ours and a removal is expected to
/// clear all of them: an earlier build — or a run that was interrupted between the key and the
/// certificate — leaves keys behind that a first-match-only deletion would never reach.
///
/// @return `Ok(empty)` when no key carries the label; `Err` when the keychain could not be
///         searched, so a failed removal surfaces instead of quietly leaving the key in place
fn search_keys() -> Result<Vec<SecKey>, CertError> {
    let mut options = ItemSearchOptions::new();
    options
        .class(ItemClass::key())
        .label(CERT_LABEL)
        .limit(Limit::All)
        .load_refs(true);

    let mut found = Vec::new();
    for result in search_items(&options, "private key")? {
        if let SearchResult::Ref(Reference::Key(key)) = result {
            found.push(key);
        }
    }
    Ok(found)
}

/// Bridges the keychain key into rcgen's `RemoteKeyPair` hook, so rcgen can build the certificate
/// while the private half stays inside the key store.
struct KeychainRemoteKey {
    /// Handle to the non-exportable keychain key; used for signing only.
    key: SecKey,
    /// SEC1 uncompressed public point, which is the format rcgen puts in SubjectPublicKeyInfo.
    public: Vec<u8>,
}

impl RemoteKeyPair for KeychainRemoteKey {
    fn public_key(&self) -> &[u8] {
        &self.public
    }

    /// Signs the certificate TBS inside the keychain.
    ///
    /// `ECDSASignatureMessageX962SHA256` returns the DER `ECDSA-Sig-Value` structure, which is
    /// exactly what an X.509 ECDSA signature BIT STRING must contain. The raw `r || s` form some
    /// APIs produce would yield a certificate no verifier accepts.
    fn sign(&self, msg: &[u8]) -> Result<Vec<u8>, rcgen::Error> {
        self.key
            .create_signature(Algorithm::ECDSASignatureMessageX962SHA256, msg)
            .map_err(|_| rcgen::Error::RemoteKeyError)
    }

    fn algorithm(&self) -> &'static SignatureAlgorithm {
        &PKCS_ECDSA_P256_SHA256
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The read results map onto the three trust states, asserted case by case rather than by
    /// example. `Deny` is the reason this is worth pinning down: collapsing it into "no grant"
    /// would let the repair path overwrite somebody's explicit refusal, and the difference is one
    /// match arm that no other test covers.
    #[test]
    fn trust_grants_map_from_read_results() {
        use TrustSettingsForCertificate::*;

        assert_eq!(trust_grant_from(Ok(Some(TrustRoot))), TrustGrant::Granted);
        assert_eq!(trust_grant_from(Ok(Some(TrustAsRoot))), TrustGrant::Granted);
        // An entry with no TLS-specific setting is the shape `set_trust_settings_always` produces,
        // because it stores a null settings array. Measured against a real keychain, a certificate
        // that reads back this way is trusted and present in `find-identity -v -p ssl-client`;
        // treating it as untrusted would rewrite the grant on every sign-in.
        assert_eq!(trust_grant_from(Ok(None)), TrustGrant::Granted);
        assert_eq!(trust_grant_from(Ok(Some(Deny))), TrustGrant::Denied);
        assert_eq!(trust_grant_from(Ok(Some(Unspecified))), TrustGrant::Missing);
        assert_eq!(trust_grant_from(Ok(Some(Invalid))), TrustGrant::Missing);
        // No entry at all is the state a build without trust handling leaves behind, and the one
        // the repair path exists to fix.
        assert_eq!(
            trust_grant_from(Err(Error::from_code(errSecItemNotFound))),
            TrustGrant::Missing
        );
    }
}
