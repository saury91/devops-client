//! Windows backend: the user's `CurrentUser\My` certificate store, through CNG.
//!
//! Chrome, Edge and Internet Explorer resolve client certificates through CryptoAPI against this
//! store, so a certificate placed here is presented with no browser-side configuration. Firefox is
//! the exception and is handled by [`super::firefox`], which flips
//! `security.osclientcerts.autoload` so Firefox reaches this same key instead of a second copy.
//!
//! The key is created in the Microsoft Software Key Storage Provider with its export policy cleared
//! before the key is finalised, so the private half cannot be exported afterwards — not by this
//! process, and not by an administrator copying the profile to another machine. That is the same
//! guarantee [`super::macos`] gets from the keychain, and it is what makes the server's fingerprint
//! check mean "this machine" rather than "whoever holds a copy of a file".
//!
//! Two details decide whether a browser can actually use the result, and both are easy to miss:
//!
//! * `NCryptCreatePersistedKey` alone leaves a key nothing points at. The certificate has to be
//!   linked to it through `CERT_KEY_PROV_INFO_PROP_ID` with `dwKeySpec = CERT_NCRYPT_KEY_SPEC` and
//!   the KSP name, because that property is what `CryptAcquireCertificatePrivateKey` follows when
//!   a browser asks for a private key. Without it the store holds a public-only certificate and the
//!   handshake silently proceeds without a client certificate.
//! * CNG emits ECDSA signatures as raw `r || s`, while an X.509 signature BIT STRING must hold a
//!   DER `ECDSA-Sig-Value`. Converting in [`ecdsa_sig_to_der`] is therefore mandatory rather than
//!   cosmetic: skipping it yields a certificate every verifier rejects.
//!
//! There is no user-visible prompt, because a software KSP key is usable by any process running as
//! the same user. Protecting it with a TPM or a PIN means `Microsoft Platform Crypto Provider` plus
//! `NCRYPT_UI_PROTECT_FLAG`, which is a deployment decision — it also changes prompt behaviour on
//! every signing operation, so it is deliberately not the default here.

use super::{
    build_self_signed, cert_info_from_der, ecdsa_sig_to_der, hex_lower, CertCapability, CertError,
    CertInfo, DeviceCertProvider, CERT_LABEL,
};
use rcgen::{RemoteKeyPair, SignatureAlgorithm, PKCS_ECDSA_P256_SHA256};
use sha2::{Digest, Sha256};
use windows_sys::Win32::Security::Cryptography::{
    CertAddEncodedCertificateToStore, CertCloseStore, CertDeleteCertificateFromStore,
    CertDuplicateCertificateContext, CertEnumCertificatesInStore, CertFreeCertificateContext,
    CertGetNameStringW, CertOpenStore, CertSetCertificateContextProperty, NCryptCreatePersistedKey,
    NCryptDeleteKey, NCryptEnumKeys, NCryptExportKey, NCryptFinalizeKey, NCryptFreeBuffer,
    NCryptFreeObject, NCryptKeyName, NCryptOpenKey, NCryptOpenStorageProvider, NCryptSetProperty,
    NCryptSignHash, BCRYPT_ECCPUBLIC_BLOB, BCRYPT_ECDSA_PUBLIC_P256_MAGIC, CERT_CONTEXT,
    CERT_KEY_PROV_INFO_PROP_ID, CERT_NAME_ATTR_TYPE, CERT_NCRYPT_KEY_SPEC,
    CERT_STORE_ADD_REPLACE_EXISTING, CERT_STORE_PROV_SYSTEM_W, CERT_SYSTEM_STORE_CURRENT_USER,
    CRYPT_KEY_PROV_INFO, HCERTSTORE, MS_KEY_STORAGE_PROVIDER, NCRYPT_ECDSA_P256_ALGORITHM,
    NCRYPT_EXPORT_POLICY_PROPERTY, NCRYPT_KEY_HANDLE, NCRYPT_OVERWRITE_KEY_FLAG,
    NCRYPT_PROV_HANDLE, PKCS_7_ASN_ENCODING, X509_ASN_ENCODING,
};

/// Store name reported to the UI and written to logs.
const STORE: &str = "CurrentUser\\My";

/// `MY` is CryptoAPI's name for the personal store, and the `CurrentUser` scope keeps everything
/// inside the profile of the user who signed in — a machine-wide install would need an elevation
/// prompt the client must not ask for.
const STORE_NAME: &str = "MY";

/// Both encoding flags are required together: `X509_ASN_ENCODING` covers the certificate itself and
/// `PKCS_7_ASN_ENCODING` the property encodings around it.
const ENCODING: u32 = X509_ASN_ENCODING | PKCS_7_ASN_ENCODING;

/// Prefix of every KSP container this client creates.
///
/// Builds that predate staged renewal used bare [`CERT_LABEL`] as the container name, so
/// [`is_our_container`] accepts that exact value too and a later [`remove`] can still clean up an
/// install left behind by such a build.
const KEY_PREFIX: &str = CERT_LABEL;

/// `2.5.4.3` — the OID of the `CN` attribute, in the NUL terminated ANSI form that
/// `CertGetNameStringW` expects for `CERT_NAME_ATTR_TYPE`.
const CN_OID: &[u8] = b"2.5.4.3\0";

/// Key store provider backed by the current user's personal certificate store.
pub struct WinCertStoreProvider;

impl DeviceCertProvider for WinCertStoreProvider {
    fn store_name(&self) -> &'static str {
        STORE
    }

    /// Always `full`: a key in `CurrentUser\My` is reachable by every browser we support, since
    /// Chrome, Edge and Internet Explorer use CryptoAPI directly and Firefox is pointed at the same
    /// key through its `osclientcerts` module. Whatever failures remain are per-user store
    /// permission problems, which [`WinCertStoreProvider::ensure`] reports as errors rather than
    /// pretending the machine is permanently limited.
    fn capability(&self) -> CertCapability {
        CertCapability::Full
    }

    fn ensure(&self, owner: &str) -> Result<CertInfo, CertError> {
        if let Some(existing) = self.newest()? {
            return Ok(existing);
        }
        self.issue_certificate(owner)
    }

    /// Issues a replacement while the certificate already installed stays usable.
    ///
    /// This is the Windows half of the staged renewal [`super::renew`] performs: the replacement
    /// lands next to the current certificate, and only after the server has accepted its fingerprint
    /// does [`DeviceCertProvider::keep_only`] drop the superseded one. One container per certificate
    /// is what makes the staging real — reusing [`KEY_PREFIX`] would overwrite the key the installed
    /// certificate reaches through `CERT_KEY_PROV_INFO_PROP_ID`, and the certificate that has to stay
    /// valid until the server acknowledges the replacement would stop working the moment the
    /// replacement was created.
    fn ensure_fresh(&self, owner: &str) -> Result<CertInfo, CertError> {
        self.issue_certificate(owner)
    }

    /// A store that cannot be opened is reported as "nothing installed" rather than as an error:
    /// the trait has no error channel here, and the value the server needs is the capability, which
    /// is what keeps a misconfigured device distinguishable from one that cannot host a certificate.
    fn status(&self) -> Option<CertInfo> {
        self.newest().ok().flatten()
    }

    fn remove(&self) -> Result<(), CertError> {
        // Both halves must go. A container left behind is dead weight the user cannot see, and the
        // next `ensure` would add yet another one; a certificate left behind keeps passing the
        // server's fingerprint check after the user signed out. Keys are matched by name prefix
        // because nothing else records which container a certificate was installed with.
        let store = open_store()?;
        let mut failure = None;
        for certificate in our_certificates(store) {
            if let Err(error) = certificate.delete() {
                failure.get_or_insert(error);
            }
        }
        unsafe { CertCloseStore(store, 0) };

        if let Err(error) = delete_all_keys() {
            failure.get_or_insert(error);
        }
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// Drops every certificate we installed except `keep_fingerprint`.
    ///
    /// Key containers are deliberately left alone. [`super::renew`] reaches this with the fingerprint
    /// the server just accepted — dropping the predecessor — or, after a registration failure, with
    /// the predecessor's fingerprint — dropping the replacement. Either way the surviving certificate
    /// still owns the key in its container, and the orphaned container is invisible to every browser
    /// because no certificate references it. [`DeviceCertProvider::remove`] sweeps them up when the
    /// device is unregistered.
    fn keep_only(&self, keep_fingerprint: &str) -> Result<(), CertError> {
        let store = open_store()?;
        let mut failure = None;
        for certificate in our_certificates(store) {
            if certificate.info.fingerprint == keep_fingerprint {
                continue;
            }
            if let Err(error) = certificate.delete() {
                failure.get_or_insert(error);
            }
        }
        unsafe { CertCloseStore(store, 0) };
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl WinCertStoreProvider {
    /// The certificate the store currently holds, if any.
    ///
    /// The newest `not_after` wins rather than the first match. Staged renewal keeps two of our
    /// certificates in the store for as long as the server round trip takes, and reporting the older
    /// one would send the client back to renew while the replacement is already installed. The
    /// comparison is on the RFC 3339 string: `format_rfc3339` emits second precision in UTC, so
    /// lexicographic order is chronological order.
    fn newest(&self) -> Result<Option<CertInfo>, CertError> {
        let store = open_store()?;
        let installed = our_certificates(store);
        unsafe { CertCloseStore(store, 0) };
        Ok(installed
            .into_iter()
            .max_by(|left, right| left.info.not_after.cmp(&right.info.not_after))
            .map(|certificate| certificate.info.clone()))
    }

    /// Creates a key in its own container and installs the certificate bound to it.
    ///
    /// The key is generated inside the KSP and only ever handled through an opaque handle; the
    /// guards release both handles on every exit path, including the error returns below.
    fn issue_certificate(&self, owner: &str) -> Result<CertInfo, CertError> {
        let container = new_container_name();
        let provider = Provider::open()?;
        let key = Key::create(&provider, &container)?;
        let public = key.public_point()?;

        // `build_self_signed` drops the remote key when it returns, so the KSP handle is released
        // as soon as signing is done.
        let issued = build_self_signed(Box::new(CngRemoteKey { key, public }), owner)?;
        // The provider handle is independent of the key handle, but the key has to outlive signing,
        // which is why it is released only now.
        drop(provider);
        install_certificate(&issued.der, &container)?;

        Ok(CertInfo {
            fingerprint: issued.fingerprint,
            serial: issued.serial,
            not_after: issued.not_after,
            capability: self.capability(),
            store: STORE,
        })
    }
}

/// Owns an `NCrypt` provider handle and releases it on every exit path.
///
/// Without it the `?` returns above would leak a handle for every failed install, in a client that
/// stays resident for weeks — a slow leak in exactly the path that runs when something is wrong.
struct Provider(NCRYPT_PROV_HANDLE);

impl Provider {
    /// Opens the software KSP: a user-scoped key that needs no TPM, smart card or elevation, and is
    /// present on every Windows install.
    fn open() -> Result<Self, CertError> {
        let mut handle: NCRYPT_PROV_HANDLE = 0;
        let hr = unsafe { NCryptOpenStorageProvider(&mut handle, MS_KEY_STORAGE_PROVIDER, 0) };
        ncheck(hr, "open Microsoft Software Key Storage Provider")?;
        Ok(Self(handle))
    }
}

impl Drop for Provider {
    fn drop(&mut self) {
        unsafe { NCryptFreeObject(self.0) };
    }
}

/// Owns a persisted key handle and releases it on every exit path.
struct Key(NCRYPT_KEY_HANDLE);

impl Key {
    /// Creates and finalises the P-256 key stored under `container`.
    ///
    /// The export policy has to be cleared while the key is still unfinalised: once
    /// `NCryptFinalizeKey` has run, the policy is part of the persisted key and can no longer be
    /// tightened from outside. That is precisely the property which keeps the private half from
    /// being exported later, so setting it to `0` before finalising is what makes device binding
    /// real rather than nominal.
    fn create(provider: &Provider, container: &[u16]) -> Result<Self, CertError> {
        let mut handle: NCRYPT_KEY_HANDLE = 0;
        // `NCRYPT_OVERWRITE_KEY_FLAG` keeps a retry after a half-finished run deterministic instead
        // of failing with NTE_EXISTS and leaving the user stuck. Safe here because every certificate
        // gets a container of its own: the only key this can ever overwrite is one that a previous
        // attempt under the same name left half-finished.
        let hr = unsafe {
            NCryptCreatePersistedKey(
                provider.0,
                &mut handle,
                NCRYPT_ECDSA_P256_ALGORITHM,
                container.as_ptr(),
                0,
                NCRYPT_OVERWRITE_KEY_FLAG,
            )
        };
        ncheck(hr, "create persisted key")?;
        let key = Self(handle);

        let not_exportable: u32 = 0;
        let hr = unsafe {
            NCryptSetProperty(
                key.0,
                NCRYPT_EXPORT_POLICY_PROPERTY,
                &not_exportable as *const u32 as *const u8,
                std::mem::size_of::<u32>() as u32,
                0,
            )
        };
        ncheck(hr, "clear key export policy")?;

        ncheck(
            unsafe { NCryptFinalizeKey(key.0, 0) },
            "finalize persisted key",
        )?;
        Ok(key)
    }

    /// Exports the public half as a SEC1 uncompressed point, the form rcgen encodes into
    /// SubjectPublicKeyInfo.
    ///
    /// CNG answers with a `BCRYPT_ECCKEY_BLOB` header (magic plus coordinate length) followed by X
    /// and Y, while SEC1 wants a leading `0x04` tag and no header. Only the public half is asked
    /// for here; the private half could not be exported even if we wanted it.
    fn public_point(&self) -> Result<Vec<u8>, CertError> {
        let mut needed: u32 = 0;
        let hr = unsafe {
            NCryptExportKey(
                self.0,
                0,
                BCRYPT_ECCPUBLIC_BLOB,
                std::ptr::null(),
                std::ptr::null_mut(),
                0,
                &mut needed,
                0,
            )
        };
        ncheck(hr, "size public key blob")?;

        let mut blob = vec![0u8; needed as usize];
        let hr = unsafe {
            NCryptExportKey(
                self.0,
                0,
                BCRYPT_ECCPUBLIC_BLOB,
                std::ptr::null(),
                blob.as_mut_ptr(),
                needed,
                &mut needed,
                0,
            )
        };
        ncheck(hr, "export public key blob")?;
        blob.truncate(needed as usize);

        if blob.len() < 8 {
            return Err(CertError::Platform(format!(
                "public key blob is {} bytes, too short for a header",
                blob.len()
            )));
        }
        let magic = u32::from_le_bytes([blob[0], blob[1], blob[2], blob[3]]);
        let coordinate_size = u32::from_le_bytes([blob[4], blob[5], blob[6], blob[7]]);
        if magic != BCRYPT_ECDSA_PUBLIC_P256_MAGIC {
            return Err(CertError::Platform(format!(
                "public key blob has unexpected magic {:#010x}",
                magic
            )));
        }
        // P-256 coordinates are 32 bytes each. Anything else means the KSP did not honour the
        // requested algorithm, and building a certificate on that key would produce one no browser
        // can match against the signature.
        if coordinate_size != 32 || blob.len() != 8 + 64 {
            return Err(CertError::Platform(format!(
                "public key blob declares {} byte coordinates in {} bytes",
                coordinate_size,
                blob.len()
            )));
        }

        let mut point = Vec::with_capacity(65);
        point.push(0x04); // SEC1 tag for an uncompressed point
        point.extend_from_slice(&blob[8..]);
        Ok(point)
    }
}

impl Drop for Key {
    fn drop(&mut self) {
        unsafe { NCryptFreeObject(self.0) };
    }
}

/// Bridges the KSP key into rcgen's `RemoteKeyPair` hook, so rcgen can assemble the certificate
/// while the private half stays inside the key store.
struct CngRemoteKey {
    /// Handle to the non-exportable key, used for signing only and released by [`Key`]'s drop.
    key: Key,
    /// SEC1 uncompressed public point, which is what rcgen puts in SubjectPublicKeyInfo.
    public: Vec<u8>,
}

impl RemoteKeyPair for CngRemoteKey {
    fn public_key(&self) -> &[u8] {
        &self.public
    }

    /// Signs the certificate TBS inside the KSP.
    ///
    /// `NCryptSignHash` takes an already computed digest — unlike the macOS API, which hashes the
    /// message itself — so the SHA-256 is taken here. The KSP answers with the raw `r || s`
    /// concatenation, which [`ecdsa_sig_to_der`] turns into the `ECDSA-Sig-Value` structure that a
    /// certificate's signature BIT STRING must contain.
    fn sign(&self, msg: &[u8]) -> Result<Vec<u8>, rcgen::Error> {
        let digest = Sha256::digest(msg);
        let hash: &[u8] = digest.as_slice();

        // Two calls, sized then filled: the signature length is not fixed by the API.
        let mut needed: u32 = 0;
        let hr = unsafe {
            NCryptSignHash(
                self.key.0,
                std::ptr::null(),
                hash.as_ptr(),
                hash.len() as u32,
                std::ptr::null_mut(),
                0,
                &mut needed,
                0,
            )
        };
        if hr < 0 {
            return Err(rcgen::Error::RemoteKeyError);
        }

        let mut signature = vec![0u8; needed as usize];
        let hr = unsafe {
            NCryptSignHash(
                self.key.0,
                std::ptr::null(),
                hash.as_ptr(),
                hash.len() as u32,
                signature.as_mut_ptr(),
                needed,
                &mut needed,
                0,
            )
        };
        if hr < 0 {
            return Err(rcgen::Error::RemoteKeyError);
        }
        signature.truncate(needed as usize);
        Ok(ecdsa_sig_to_der(signature))
    }

    fn algorithm(&self) -> &'static SignatureAlgorithm {
        &PKCS_ECDSA_P256_SHA256
    }
}

/// Opens the current user's personal store for reading and writing.
fn open_store() -> Result<HCERTSTORE, CertError> {
    let mut name = wide(STORE_NAME);
    let store = unsafe {
        CertOpenStore(
            CERT_STORE_PROV_SYSTEM_W,
            0,
            0,
            CERT_SYSTEM_STORE_CURRENT_USER,
            name.as_mut_ptr() as *const core::ffi::c_void,
        )
    };
    if store.is_null() {
        return Err(platform_error("open CurrentUser\\My certificate store"));
    }
    Ok(store)
}

/// One of our certificates held by the store, with its context owned.
///
/// The context is a duplicate rather than the pointer the enumeration handed out. Both enumeration
/// and deletion release contexts — `CertEnumCertificatesInStore` frees the one passed back to it,
/// and `CertDeleteCertificateFromStore` frees the one it is given — so operating on the
/// enumeration's own pointer would either cut the walk short or free the same context twice.
struct OurCert {
    /// `None` once ownership has been handed over; see [`OurCert::delete`].
    context: Option<*mut CERT_CONTEXT>,
    info: CertInfo,
}

impl OurCert {
    /// Removes the certificate from its store.
    ///
    /// `CertDeleteCertificateFromStore` frees the context it is given **even when it fails** (MSDN),
    /// so the handle is moved out of `self` first and [`Drop`] must not free it a second time.
    fn delete(mut self) -> Result<(), CertError> {
        let context = self.context.take().expect("context is taken exactly once");
        if unsafe { CertDeleteCertificateFromStore(context) } == 0 {
            return Err(platform_error("delete certificate from store"));
        }
        Ok(())
    }
}

impl Drop for OurCert {
    fn drop(&mut self) {
        if let Some(context) = self.context.take() {
            unsafe { CertFreeCertificateContext(context) };
        }
    }
}

/// Collects every certificate of ours from `store`.
///
/// Certificates are picked by their `CN` rather than by a subject sub-string search, so a certificate
/// belonging to somebody else is never selected for deletion. Entries whose DER cannot be parsed are
/// skipped: they cannot be described to the server in the first place.
fn our_certificates(store: HCERTSTORE) -> Vec<OurCert> {
    let mut found = Vec::new();
    // Each context is handed straight back on the next iteration: that call frees the previous one,
    // on failure too, so the loop never frees a context itself.
    let mut previous: *const CERT_CONTEXT = std::ptr::null();
    loop {
        let current = unsafe { CertEnumCertificatesInStore(store, previous) };
        if current.is_null() {
            break;
        }
        if common_name(current).as_deref() == Some(CERT_LABEL) {
            let der = encoded_certificate(current);
            if let Some(info) = cert_info_from_der(&der, CertCapability::Full, STORE) {
                found.push(OurCert {
                    context: Some(unsafe { CertDuplicateCertificateContext(current) }),
                    info,
                });
            }
        }
        previous = current;
    }
    found
}

/// Reads the certificate's `CN`, the field every backend keys its own certificates on.
///
/// The machine-specific owner lives in the `OU`, so the `CN` is the stable half of the subject and
/// the only part compared against [`CERT_LABEL`].
fn common_name(context: *const CERT_CONTEXT) -> Option<String> {
    // Two calls: the first measures, the second fills. `CN_OID` is a NUL terminated ANSI OID string,
    // which is the form `CERT_NAME_ATTR_TYPE` expects.
    let needed = unsafe {
        CertGetNameStringW(
            context,
            CERT_NAME_ATTR_TYPE,
            0,
            CN_OID.as_ptr() as *const core::ffi::c_void,
            std::ptr::null_mut(),
            0,
        )
    };
    // A single character means "the terminator only", i.e. the certificate carries no such attribute.
    if needed <= 1 {
        return None;
    }

    let mut buffer = vec![0u16; needed as usize];
    let written = unsafe {
        CertGetNameStringW(
            context,
            CERT_NAME_ATTR_TYPE,
            0,
            CN_OID.as_ptr() as *const core::ffi::c_void,
            buffer.as_mut_ptr(),
            needed,
        )
    };
    if written == 0 {
        return None;
    }
    let end = buffer
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..end]))
}

/// Names a fresh KSP container for a single certificate.
///
/// The per-certificate suffix is what lets two of our certificates coexist, and it is random rather
/// than derived from the certificate because the name must exist before the certificate it will hold
/// is signed.
fn new_container_name() -> Vec<u16> {
    use rand::RngCore;
    let mut suffix = [0u8; 8];
    rand::rngs::OsRng.fill_bytes(&mut suffix);
    wide(&format!("{}-{}", KEY_PREFIX, hex_lower(&suffix)))
}

/// Whether a KSP container name belongs to this client.
///
/// The bare prefix counts too, so a container written by a build that predates staged renewal is
/// still swept up by [`delete_all_keys`].
fn is_our_container(name: &str) -> bool {
    name == KEY_PREFIX
        || name
            .strip_prefix(KEY_PREFIX)
            .is_some_and(|rest| rest.starts_with('-'))
}

/// Copies the DER bytes out of a certificate context.
///
/// The context already points at the encoded certificate, which avoids a property query that may
/// not be answered for certificates written by an older build.
fn encoded_certificate(context: *const CERT_CONTEXT) -> Vec<u8> {
    let context = unsafe { &*context };
    if context.pbCertEncoded.is_null() || context.cbCertEncoded == 0 {
        return Vec::new();
    }
    unsafe {
        std::slice::from_raw_parts(context.pbCertEncoded, context.cbCertEncoded as usize).to_vec()
    }
}

/// Adds the certificate to the store and links it to the KSP key.
///
/// The link is the part that decides whether this works at all: `CryptAcquireCertificatePrivateKey`
/// — the call every browser makes when the server asks for a client certificate — resolves a
/// private key through `CERT_KEY_PROV_INFO_PROP_ID`. A certificate added without that property is
/// public-only, and the handshake then quietly proceeds as if the device had no certificate.
/// `CERT_NCRYPT_KEY_SPEC` in `dwKeySpec` is what directs Windows to a CNG provider rather than to a
/// legacy CSP, which is where the key actually lives.
fn install_certificate(der: &[u8], container: &[u16]) -> Result<(), CertError> {
    let store = open_store()?;
    let mut context: *mut CERT_CONTEXT = std::ptr::null_mut();
    let added = unsafe {
        CertAddEncodedCertificateToStore(
            store,
            ENCODING,
            der.as_ptr(),
            der.len() as u32,
            CERT_STORE_ADD_REPLACE_EXISTING,
            &mut context,
        )
    };
    if added == 0 || context.is_null() {
        unsafe { CertCloseStore(store, 0) };
        return Err(platform_error("add certificate to CurrentUser\\My"));
    }

    // `CRYPT_KEY_PROV_INFO` declares its strings as writable, so the container name is copied into a
    // buffer that outlives the call setting the property.
    let mut name = container.to_vec();
    let info = CRYPT_KEY_PROV_INFO {
        pwszContainerName: name.as_mut_ptr(),
        // Taken straight from the windows-sys binding rather than repeated as a literal, so the two
        // cannot drift apart. The cast only drops `const`: the API reads this static string.
        pwszProvName: MS_KEY_STORAGE_PROVIDER as *mut u16,
        dwProvType: 0, // unused by a CNG provider, but the struct is shared with legacy CSPs
        dwFlags: 0,
        cProvParam: 0,
        rgProvParam: std::ptr::null_mut(),
        dwKeySpec: CERT_NCRYPT_KEY_SPEC,
    };
    let linked = unsafe {
        CertSetCertificateContextProperty(
            context,
            CERT_KEY_PROV_INFO_PROP_ID,
            0,
            &info as *const CRYPT_KEY_PROV_INFO as *const core::ffi::c_void,
        )
    };
    if linked == 0 {
        // Withdraw the certificate that was just added. Leaving it behind would make `status` report
        // an identity whose private key no browser can reach: under `cert-mode=enforce` that is worse
        // than having no certificate at all, because the device looks bound while every handshake
        // fails. Also required by staged renewal — a certificate without a key is a leftover nobody
        // can sign with. `CertDeleteCertificateFromStore` frees the context itself, failure included.
        unsafe { CertDeleteCertificateFromStore(context) };
        unsafe { CertCloseStore(store, 0) };
        return Err(platform_error("link certificate to the KSP key"));
    }

    // Ours to release: the context returned above is a copy, not a store handle.
    unsafe { CertFreeCertificateContext(context) };
    unsafe { CertCloseStore(store, 0) };
    Ok(())
}

/// Deletes every KSP container this client created.
///
/// The names have to be enumerated rather than derived: each certificate gets a container of its own
/// and nothing on the certificate records which one that was. [`is_our_container`] keeps the sweep
/// from touching a key that belongs to somebody else in the same provider.
fn delete_all_keys() -> Result<(), CertError> {
    let provider = Provider::open()?;
    let mut state: *mut core::ffi::c_void = std::ptr::null_mut();
    let mut failure = None;

    loop {
        let mut key: *mut NCryptKeyName = std::ptr::null_mut();
        let hr = unsafe { NCryptEnumKeys(provider.0, std::ptr::null(), &mut key, &mut state, 0) };
        // Any negative result ends the walk: `NTE_NO_MORE_ITEMS` is simply the provider running out.
        if hr < 0 {
            break;
        }

        let name = unsafe { wide_string((*key).pszName) };
        // The buffer belongs to the provider and has to go back before the next iteration.
        unsafe { NCryptFreeBuffer(key as *mut core::ffi::c_void) };

        if !is_our_container(&name) {
            continue;
        }
        if let Err(error) = delete_key_by_name(&provider, &wide(&name)) {
            failure.get_or_insert(error);
        }
    }

    // Released even when the loop ended on an error, otherwise the provider leaks the state.
    if !state.is_null() {
        unsafe { NCryptFreeBuffer(state) };
    }

    match failure {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Deletes one persisted key by name, treating its absence as success.
///
/// The handle is deliberately not wrapped in [`Key`]: `NCryptDeleteKey` frees the handle itself on
/// success, so the guard would free it a second time. On failure the handle is still ours and has
/// to be released here, which is why both branches are spelled out.
fn delete_key_by_name(provider: &Provider, name: &[u16]) -> Result<(), CertError> {
    let mut handle: NCRYPT_KEY_HANDLE = 0;
    let hr = unsafe { NCryptOpenKey(provider.0, &mut handle, name.as_ptr(), 0, 0) };
    if hr < 0 {
        // Already gone, which is the state this function exists to reach — a machine that never
        // installed a certificate, or one whose key a previous sweep already removed.
        return Ok(());
    }

    let hr = unsafe { NCryptDeleteKey(handle, 0) };
    if hr < 0 {
        unsafe { NCryptFreeObject(handle) };
        return Err(platform_error("delete persisted key"));
    }
    Ok(())
}

/// Copies a NUL terminated UTF-16 string out of a buffer owned by the calling API.
///
/// Used for the names `NCryptEnumKeys` returns: those bytes are released with `NCryptFreeBuffer`
/// instead of through `Drop`, so nothing here may take ownership of the pointer.
unsafe fn wide_string(value: *const u16) -> String {
    if value.is_null() {
        return String::new();
    }
    let mut length = 0usize;
    while unsafe { *value.add(length) } != 0 {
        length += 1;
    }
    unsafe { String::from_utf16_lossy(std::slice::from_raw_parts(value, length)) }
}

/// NUL terminated UTF-16, the encoding every `PCWSTR` parameter expects.
///
/// Built here rather than with `windows_sys::core::w!` because [`open_store`] hands out a mutable
/// pointer: `CertOpenStore` receives the store name as a `*const c_void` carved out of `PWSTR`
/// storage.
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Turns an `NCrypt`/`CryptoAPI` failure into a message carrying the system error text, which is
/// the only way an operator can tell a locked profile from a missing key.
fn platform_error(what: &str) -> CertError {
    CertError::Platform(format!("{}: {}", what, std::io::Error::last_os_error()))
}

/// Fails unless `hr` reports success.
///
/// These APIs return HRESULTs, where every negative value is a failure code such as `NTE_BAD_KEY`
/// or `NTE_EXISTS`, so the sign bit is the whole test.
fn ncheck(hr: windows_sys::core::HRESULT, what: &str) -> Result<(), CertError> {
    if hr < 0 {
        Err(platform_error(what))
    } else {
        Ok(())
    }
}
