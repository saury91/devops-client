//! Client TLS certificate: the device's cryptographic identity for browser sessions.
//!
//! Why the key is generated inside the OS key store: the browser presents this certificate
//! during the TLS handshake, so the server can tell "this request came from this machine"
//! without trusting anything the page or the local client says. That guarantee only holds if
//! the private half cannot leave the machine — shipping a PFX/P12 file for the user to install
//! would put an exportable private key on disk, and copying that file to another computer would
//! defeat device binding entirely. Every backend below therefore asks the operating system to
//! create a non-exportable key and only ever reports the public fingerprint upstream.
//!
//! Capability levels exist so the server can enforce certificates per device rather than
//! all-or-nothing, which is what keeps older or restricted machines from being locked out:
//!
//! | Capability    | Meaning                                                              |
//! |---------------|----------------------------------------------------------------------|
//! | `full`        | Key lives in the OS key store; every supported browser presents it.   |
//! | `partial`     | A certificate is installed, but not every browser presents it — which  |
//! |               | includes one the OS does not trust, since no browser presents that.    |
//! | `unavailable` | This machine cannot host one; the server keeps the legacy checks.     |
//!
//! Which of the three a machine reports is decided in [`reported_capability`], which prefers what
//! the installed certificate can actually do over what the platform is capable of. [`capability`]
//! is only the ceiling, and is what that answer falls back to while nothing is installed.

use serde::Serialize;

// Not platform gated: profile discovery is plain file work, and each platform's own stage decides
// whether a `user.js` preference can help it or whether the certificate has to go into NSS instead.
pub mod firefox;
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod win;

/// Label / key-store alias identifying our certificate, so repeated runs find the same one
/// instead of piling up duplicates in the user's key store.
pub const CERT_LABEL: &str = "devops-client";

/// Validity of a freshly issued certificate. Kept below 398 days — the ceiling browsers apply
/// to publicly trusted certificates — so a client certificate can never be rejected for
/// lifetime alone. Renewal starts 30 days before expiry.
pub const VALIDITY_DAYS: i64 = 397;

/// Days before expiry at which the client re-issues its certificate.
///
/// The margin exists for machines that stay switched off: without it a laptop closed for a month
/// would come back holding an already-expired certificate, able to present nothing at the TLS
/// handshake, and would need a fresh sign-in at exactly the moment the server is enforcing.
pub const RENEW_BEFORE_DAYS: i64 = 30;

/// How well this machine can participate in client-certificate authentication.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CertCapability {
    /// Key resides in the OS key store and every supported browser presents it.
    Full,
    /// A certificate exists, but not every browser can present it.
    Partial,
    /// No certificate can be provided on this machine.
    Unavailable,
}

impl CertCapability {
    /// Lowercase wire value, matching `app.device-binding` on the server.
    pub fn as_str(self) -> &'static str {
        match self {
            CertCapability::Full => "full",
            CertCapability::Partial => "partial",
            CertCapability::Unavailable => "unavailable",
        }
    }
}

/// Metadata describing an installed client certificate.
#[derive(Debug, Clone, Serialize)]
pub struct CertInfo {
    /// SHA-256 of the DER certificate as lowercase hex — the value the server stores.
    pub fingerprint: String,
    /// Certificate serial as hex, for operator troubleshooting only.
    pub serial: String,
    /// Not-after instant in RFC 3339, used for renewal prompts and server-side expiry hints.
    pub not_after: String,
    /// Capability of the store holding this certificate.
    pub capability: CertCapability,
    /// Human readable store name, shown in the UI and written to logs. Static because every
    /// backend reports a compile-time constant name, which keeps the struct cheap to clone.
    pub store: &'static str,
}

/// Why a certificate operation failed.
#[derive(Debug, Clone)]
pub enum CertError {
    /// This platform or configuration cannot host a client certificate.
    Unsupported(String),
    /// The operating system rejected the operation (locked key store, denied access, ...).
    Platform(String),
}

impl std::fmt::Display for CertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CertError::Unsupported(detail) => write!(f, "UNSUPPORTED: {}", detail),
            CertError::Platform(detail) => write!(f, "PLATFORM: {}", detail),
        }
    }
}

/// A platform key store able to issue and hold the client certificate.
pub trait DeviceCertProvider: Send + Sync {
    /// Human readable store name, e.g. `login keychain` or `CurrentUser\My`.
    fn store_name(&self) -> &'static str;

    /// Best capability this machine can reach, decided before any certificate exists.
    fn capability(&self) -> CertCapability;

    /// Returns the installed certificate, creating and installing one when absent.
    ///
    /// `owner` is written into the `OU` field so an operator triaging the key store by hand can
    /// tell which user and machine a certificate belongs to. The `CN` stays fixed at
    /// [`CERT_LABEL`] because every backend locates its certificate by that subject text — making
    /// it instance specific would force a full-certificate scan on each launch.
    fn ensure(&self, owner: &str) -> Result<CertInfo, CertError>;

    /// Looks up an already installed certificate without creating one.
    fn status(&self) -> Option<CertInfo>;

    /// Settles the trust of the certificate already installed, without installing one.
    ///
    /// Only macOS separates trust from installation, so the default does nothing and the other
    /// backends are complete without an implementation.
    ///
    /// Kept apart from [`DeviceCertProvider::ensure`] so it is safe on a periodic path: never
    /// creating a certificate means it cannot leave behind the state `ensure` avoids by doing both
    /// halves at once — a device holding an identity the server has not been told about.
    ///
    /// @return `Ok(())` whether or not there was anything to settle; `Err` only when the key store
    ///         could not be read
    fn repair_installed(&self) -> Result<(), CertError> {
        Ok(())
    }

    /// Removes the installed certificate and its key. Used when the user signs out for good
    /// or re-registers, so a stale identity cannot keep passing the server's fingerprint check.
    fn remove(&self) -> Result<(), CertError>;

    /// Installs a replacement certificate while the current one stays usable.
    ///
    /// Distinct from [`DeviceCertProvider::ensure`], which hands back whatever is already installed:
    /// that is what keeps signing in idempotent, and it also means `ensure` can never produce a
    /// replacement. The returned certificate is the one to register upstream.
    ///
    /// The staging is the point. A renewal happens while the device is bound, so a backend that
    /// drops the old certificate before it knows the new one can be installed leaves the machine
    /// with no identity at all and, under `cert-mode=enforce`, no way to open a browser session
    /// until the user signs in again. Callers drop the superseded certificate afterwards, through
    /// [`DeviceCertProvider::keep_only`].
    ///
    /// The default keeps the previous, un-staged sequence so a backend that cannot hold two of its
    /// certificates at once — or one that has never been taught to stage — behaves as before.
    fn ensure_fresh(&self, owner: &str) -> Result<CertInfo, CertError> {
        self.remove()?;
        self.ensure(owner)
    }

    /// Drops every certificate of ours except `keep_fingerprint`.
    ///
    /// The caller's one decision — which fingerprint is the device — is the argument, so the same
    /// primitive serves both directions of a renewal: keep the replacement to finish it, or keep the
    /// superseded certificate to undo it after the server refused the replacement.
    ///
    /// Kept separate from the install so that a failure here cannot undo an installed certificate:
    /// by the time it is called the kept certificate is in the key store and registered upstream,
    /// which makes a leftover a degraded end state rather than a device that cannot authenticate.
    ///
    /// The default does nothing, which matches a backend whose `ensure_fresh` already removed the
    /// old certificate before installing the replacement.
    fn keep_only(&self, keep_fingerprint: &str) -> Result<(), CertError> {
        let _ = keep_fingerprint;
        Ok(())
    }
}

/// Material for a self-signed certificate produced by [`build_self_signed`].
///
/// Used by the macOS and Windows backends only. The shared helpers stay in this file — rather than
/// in each backend — so their tests run on every platform, which leaves them dead code on Linux.
/// That is why the lint is silenced per item here, exactly as [`ecdsa_sig_to_der`] does it.
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
pub(crate) struct IssuedCert {
    /// DER encoded certificate.
    pub der: Vec<u8>,
    /// SHA-256 of `der` as lowercase hex.
    pub fingerprint: String,
    /// Serial as hex, taken from the very bytes embedded in the certificate.
    pub serial: String,
    /// Not-after instant in RFC 3339.
    pub not_after: String,
}

/// DER encodes a self-signed client certificate, signing it through `remote_key`.
///
/// rcgen is used strictly as an encoder: it hands the TBS bytes to `remote_key`, which forwards
/// them to the OS key store for signing. No private key material is ever materialised here, and
/// the `RemoteKeyPair` indirection is exactly the hook rcgen provides for hardware/OS held keys.
///
/// The signature bytes are used verbatim as the certificate's BIT STRING, so a backend must
/// return a DER-encoded `ECDSA-Sig-Value` — not the raw `r || s` concatenation some APIs emit.
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
pub(crate) fn build_self_signed(
    remote_key: Box<dyn rcgen::RemoteKeyPair + Send + Sync>,
    owner: &str,
) -> Result<IssuedCert, CertError> {
    let key_pair = rcgen::KeyPair::from_remote(remote_key)
        .map_err(|e| CertError::Platform(format!("load key store key: {}", e)))?;

    let mut params = rcgen::CertificateParams::default();
    // CN is the stable lookup handle for every backend; the owner goes to OU for triage.
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, CERT_LABEL);
    if !owner.is_empty() {
        params
            .distinguished_name
            .push(rcgen::DnType::OrganizationalUnitName, owner);
    }
    params
        .distinguished_name
        .push(rcgen::DnType::OrganizationName, "fw-devops");

    // A random serial keeps the certificate unique across re-issues; some stores silently
    // overwrite an existing certificate when the serial repeats.
    let serial_bytes = {
        use rand::RngCore;
        let mut buf = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut buf);
        buf[0] &= 0x7f; // keep the DER INTEGER positive
        buf.to_vec()
    };
    params.serial_number = Some(rcgen::SerialNumber::from_slice(&serial_bytes));

    let now = to_second(time::OffsetDateTime::now_utc());
    // Kept in a local because `self_signed` consumes `params`.
    let not_after = now + time::Duration::days(VALIDITY_DAYS);
    params.not_before = now - time::Duration::hours(1); // tolerate a slightly slow clock
    params.not_after = not_after;

    let cert = params
        .self_signed(&key_pair)
        .map_err(|e| CertError::Platform(format!("self-sign certificate: {}", e)))?;
    let der = cert.der().to_vec();

    Ok(IssuedCert {
        fingerprint: sha256_hex(&der),
        serial: hex_lower(&serial_bytes),
        not_after: format_rfc3339(not_after),
        der,
    })
}

/// SHA-256 digest of `bytes` as lowercase hex; the fingerprint format the server compares.
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_lower(&hasher.finalize())
}

/// Lowercase hex encoding without pulling in another crate.
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
pub(crate) fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{:02x}", b));
    }
    out
}

/// Converts a raw ECDSA signature into the DER `ECDSA-Sig-Value` a certificate signature must hold.
///
/// Windows CNG returns the two integers concatenated (`r || s`, fixed width, big-endian), whereas an
/// X.509 signature BIT STRING has to contain a DER `SEQUENCE` of two `INTEGER`s. Because
/// [`build_self_signed`] uses the returned bytes verbatim, a backend that skips this step produces a
/// certificate no verifier accepts — and the symptom is a handshake rejection, which points
/// everywhere except at the signature encoding. The other backends happen to emit DER already, and
/// this function leaves anything that is not two 32-byte integers untouched.
///
/// That length test is sound rather than a heuristic: the shortest DER encoding of this structure is
/// 68 bytes, so a 64-byte input can only be the raw concatenation.
///
/// Only the Windows backend calls this, but it lives here rather than in `win.rs` so its tests can
/// run on every platform: `win.rs` is compiled for Windows alone, and an encoding mistake inside it
/// would otherwise stay invisible until it reached a real machine.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn ecdsa_sig_to_der(signature: Vec<u8>) -> Vec<u8> {
    if signature.len() != 64 {
        return signature;
    }

    let r = der_integer(&signature[..32]);
    let s = der_integer(&signature[32..]);
    let mut out = Vec::with_capacity(2 + r.len() + s.len());
    out.push(0x30); // SEQUENCE
                    // Two integers of at most 33 bytes keep the body under 128 bytes, so the short length form
                    // always applies and no long form is needed here.
    out.push((r.len() + s.len()) as u8);
    out.extend_from_slice(&r);
    out.extend_from_slice(&s);
    out
}

/// DER `INTEGER` for an unsigned big-endian value, the form ECDSA components are defined in.
///
/// Unused outside Windows for the same reason as [`ecdsa_sig_to_der`].
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn der_integer(bytes: &[u8]) -> Vec<u8> {
    // Leading zeros are not part of the value; a single zero byte is all that remains when the
    // value is zero, which is a legal signature component.
    let start = bytes
        .iter()
        .position(|b| *b != 0)
        .unwrap_or(bytes.len().saturating_sub(1));
    let value = &bytes[start..];

    // Without this a component whose top bit is set would decode as a negative number.
    let pad = match value.first() {
        Some(b) if b & 0x80 != 0 => 1,
        _ => 0,
    };

    let mut out = Vec::with_capacity(2 + pad + value.len());
    out.push(0x02); // INTEGER
    out.push((pad + value.len()) as u8);
    if pad == 1 {
        out.push(0);
    }
    out.extend_from_slice(value);
    out
}

/// Reads back the serial number and not-after instant from a DER certificate.
///
/// Needed by [`DeviceCertProvider::status`]: on a later launch only the stored DER is available,
/// and the server wants both values so it can show expiry and match the certificate in its own
/// records. Parsing beats caching them in a side file — a cache can drift from the certificate
/// actually installed in the key store, and then the UI lies about which certificate is in use.
///
/// @return `(serial hex, not-after RFC 3339)`, or `None` when the DER is not a certificate
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
pub(crate) fn parse_cert_meta(der: &[u8]) -> Option<(String, String)> {
    // Certificate ::= SEQUENCE { tbsCertificate, signatureAlgorithm, signatureValue }
    let mut outer = DerReader::new(der);
    let (tag, certificate) = outer.tlv()?;
    if tag != 0x30 {
        return None;
    }
    let (tag, tbs) = DerReader::new(certificate).tlv()?;
    if tag != 0x30 {
        return None;
    }

    let mut body = DerReader::new(tbs);
    let mut field = body.tlv()?;
    // Optional explicit [0] version; present in every v3 certificate we emit.
    if field.0 == 0xa0 {
        field = body.tlv()?;
    }
    if field.0 != 0x02 {
        return None;
    }
    let serial = hex_lower(field.1);

    body.tlv()?; // signature AlgorithmIdentifier
    body.tlv()?; // issuer Name
    let (tag, validity) = body.tlv()?;
    if tag != 0x30 {
        return None;
    }
    let mut times = DerReader::new(validity);
    times.tlv()?; // notBefore
    let (tag, not_after) = times.tlv()?;
    Some((serial, parse_asn1_time(tag, not_after)?))
}

/// Decodes an ASN.1 UTCTime (`0x17`) or GeneralizedTime (`0x18`) into RFC 3339, both of which
/// are always expressed as UTC in certificates.
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
fn parse_asn1_time(tag: u8, value: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(value).ok()?;
    let digits = text.strip_suffix('Z')?;
    let (year, rest) = match tag {
        0x17 => {
            // UTCTime: YYMMDDHHMMSS, where 50..99 means 1950..1999 and 00..49 means 2000..2049.
            let yy: i32 = digits.get(0..2)?.parse().ok()?;
            (
                if yy >= 50 { 1900 + yy } else { 2000 + yy },
                digits.get(2..)?,
            )
        }
        0x18 => {
            let year: i32 = digits.get(0..4)?.parse().ok()?;
            (year, digits.get(4..)?)
        }
        _ => return None,
    };
    if rest.len() < 10 {
        return None;
    }
    let month: u8 = rest.get(0..2)?.parse().ok()?;
    let day: u8 = rest.get(2..4)?.parse().ok()?;
    let hour: u8 = rest.get(4..6)?.parse().ok()?;
    let minute: u8 = rest.get(6..8)?.parse().ok()?;
    let second: u8 = rest.get(8..10)?.parse().ok()?;

    let date =
        time::Date::from_calendar_date(year, time::Month::try_from(month).ok()?, day).ok()?;
    let clock = time::Time::from_hms(hour, minute, second).ok()?;
    Some(format_rfc3339(
        time::PrimitiveDateTime::new(date, clock).assume_utc(),
    ))
}

/// Minimal definite-length DER reader; just enough to pull two fields out of a certificate.
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
struct DerReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
impl<'a> DerReader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn byte(&mut self) -> Option<u8> {
        let b = *self.buf.get(self.pos)?;
        self.pos += 1;
        Some(b)
    }

    fn take(&mut self, count: usize) -> Option<&'a [u8]> {
        let slice = self.buf.get(self.pos..self.pos.checked_add(count)?)?;
        self.pos += count;
        Some(slice)
    }

    fn length(&mut self) -> Option<usize> {
        let first = self.byte()?;
        if first & 0x80 == 0 {
            return Some(first as usize);
        }
        // Long form: the low bits give the byte count of the length itself.
        let count = (first & 0x7f) as usize;
        if count == 0 || count > 4 {
            return None;
        }
        let mut value = 0usize;
        for _ in 0..count {
            value = (value << 8) | self.byte()? as usize;
        }
        Some(value)
    }

    /// Reads one tag-length-value triple, returning its tag and value bytes.
    fn tlv(&mut self) -> Option<(u8, &'a [u8])> {
        let tag = self.byte()?;
        let length = self.length()?;
        let value = self.take(length)?;
        Some((tag, value))
    }
}

/// Drops sub-second precision, which X.509 time fields cannot carry.
///
/// Applied before signing so the `not_after` we report upstream is byte-for-byte the value inside
/// the certificate, keeping the server's renewal decision and the certificate's own expiry in step.
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
fn to_second(value: time::OffsetDateTime) -> time::OffsetDateTime {
    value.replace_nanosecond(0).unwrap_or(value)
}

/// RFC 3339 rendering of `value`, the format the server's `parseCertNotAfter` understands.
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
fn format_rfc3339(value: time::OffsetDateTime) -> String {
    value
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| value.unix_timestamp().to_string())
}

/// The key store implementation for the current operating system.
pub fn provider() -> &'static dyn DeviceCertProvider {
    #[cfg(target_os = "macos")]
    {
        &macos::MacKeychainProvider
    }
    #[cfg(target_os = "windows")]
    {
        &win::WinCertStoreProvider
    }
    #[cfg(target_os = "linux")]
    {
        &linux::NssProvider
    }
}

/// Ceiling this machine can reach, whether or not a certificate exists yet.
///
/// A platform property, not a statement about the certificate in place: macOS and Windows are
/// `full` even on the day nothing has been installed. That is what makes it the right input for
/// telling a misconfigured device (`full`, no certificate) from one that cannot host one at all
/// (`unavailable`) — and the wrong thing to report as the machine's current state, which is
/// [`reported_capability`].
pub fn capability() -> CertCapability {
    provider().capability()
}

/// Capability describing what this machine can actually do right now.
///
/// [`capability`] alone cannot express "a certificate is installed that no browser will present",
/// which is exactly the state an upgrade from a trust-less build lands in: the machine would report
/// `full`, the same value a working one reports, and nothing upstream could tell them apart. The
/// installed certificate therefore decides the answer, and the ceiling is used only while nothing is
/// installed.
pub fn reported_capability() -> CertCapability {
    effective_capability(&status())
}

/// Effective capability of a status: the installed certificate's own capability when there is one,
/// the platform ceiling otherwise.
///
/// Split out from [`reported_capability`] so the choice is testable without a key store, which is
/// what pins the case that matters — an installed `partial` must not be reported as the platform's
/// `full`.
fn effective_capability(status: &CertStatus) -> CertCapability {
    status
        .installed
        .as_ref()
        .map_or(status.capability, |info| info.capability)
}

/// Settles the trust of an already installed certificate, installing nothing.
///
/// For the paths that must not install: the heartbeat, and the moment before a browser session is
/// handed the certificate. A machine upgraded from a build that never granted trust holds a
/// certificate no browser will present, and neither a silent (`auto_login`) sign-in nor the
/// heartbeat goes through [`ensure`], so without this such a machine would stay broken for as long
/// as it keeps its session and never types a password again.
///
/// @return `Ok(())` when there was nothing to settle or it was settled; `Err` when the key store
///         could not be read
pub fn repair_installed() -> Result<(), CertError> {
    provider().repair_installed()
}

/// Certificate state of this machine: what it can do, and what is installed right now.
#[derive(Debug, Clone, Serialize)]
pub struct CertStatus {
    /// Best capability this machine can reach, whether or not a certificate exists yet.
    pub capability: CertCapability,
    /// Key store holding (or able to hold) the certificate, shown in the UI for diagnosis.
    pub store: &'static str,
    /// Installed certificate; absent when the machine has none yet.
    pub installed: Option<CertInfo>,
}

/// Reads the current certificate state.
///
/// Never fails: an empty or unreadable key store is reported as "nothing installed" together with
/// the machine's capability, which is what lets the server tell a misconfigured device apart from
/// one that cannot host a certificate at all.
pub fn status() -> CertStatus {
    let provider = provider();
    CertStatus {
        capability: provider.capability(),
        store: provider.store_name(),
        installed: provider.status(),
    }
}

/// Installs the certificate when absent, returning the resulting state.
///
/// Returns the full state rather than `()` so the caller can report the certificate upstream
/// without a second key-store round trip; a second lookup could observe a different certificate
/// and send a fingerprint that does not match the one just installed.
pub fn ensure(owner: &str) -> Result<CertStatus, CertError> {
    let provider = provider();
    let installed = provider.ensure(owner)?;
    Ok(CertStatus {
        capability: installed.capability,
        store: installed.store,
        installed: Some(installed),
    })
}

/// Removes the installed certificate together with its private key.
pub fn remove() -> Result<(), CertError> {
    provider().remove()
}

/// Installs a replacement certificate, leaving the current one in the key store.
///
/// Half of a renewal, and deliberately only half: the caller still has to register the returned
/// certificate upstream and then call [`keep_only`] with its fingerprint. Splitting them is what
/// lets a renewal come back from a refused registration — the certificate the server has on record
/// is still installed at this point, so dropping the fresh one restores the previous state exactly.
///
/// Nothing is registered here, and the startup path must never install for a machine the server has
/// not been told about; the call sites are the heartbeat's renewal window and the UI's retry button.
pub fn install_replacement(owner: &str) -> Result<CertInfo, CertError> {
    provider().ensure_fresh(owner)
}

/// Drops every certificate of ours except `keep_fingerprint`.
///
/// Binds the trait method that owns the key-store semantics, so callers do not have to know which
/// backend implements it — or that they are being asked to keep a superseded certificate while
/// undoing an install that the server refused.
pub fn keep_only(keep_fingerprint: &str) -> Result<(), CertError> {
    provider().keep_only(keep_fingerprint)
}

/// Whether the installed certificate is close enough to expiry to be re-issued.
pub fn needs_renewal(info: &CertInfo) -> bool {
    needs_renewal_at(&info.not_after, time::OffsetDateTime::now_utc())
}

/// [`needs_renewal`] with an explicit "now", so the boundary behaviour stays testable.
///
/// @param not_after RFC 3339 expiry as reported by the key store
/// @param now       当前时间
/// @return 是否应进入续期窗口
pub(crate) fn needs_renewal_at(not_after: &str, now: time::OffsetDateTime) -> bool {
    match time::OffsetDateTime::parse(not_after, &time::format_description::well_known::Rfc3339) {
        Ok(expiry) => expiry - now < time::Duration::days(RENEW_BEFORE_DAYS),
        // An expiry we cannot read must not silently disable renewal for good.
        Err(_) => true,
    }
}

/// Assembles [`CertInfo`] from a DER certificate held by a store of the given capability.
///
/// @return `None` when `der` cannot be parsed, so a corrupt key-store entry degrades to
///         "no certificate" instead of surfacing a half-filled record to the server
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
pub(crate) fn cert_info_from_der(
    der: &[u8],
    capability: CertCapability,
    store: &'static str,
) -> Option<CertInfo> {
    let (serial, not_after) = parse_cert_meta(der)?;
    Some(CertInfo {
        fingerprint: sha256_hex(der),
        serial,
        not_after,
        capability,
        store,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The serial and not-after read back from DER must match what the certificate was created
    /// with: `status()` relies on this to report the certificate actually installed, and the
    /// server uses the same values to decide when to prompt for renewal.
    #[test]
    fn parse_cert_meta_matches_generated_certificate() {
        let key_pair = rcgen::KeyPair::generate().expect("generate key");
        let mut params = rcgen::CertificateParams::default();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "test-device");
        let serial: Vec<u8> = (1u8..=16).collect();
        params.serial_number = Some(rcgen::SerialNumber::from_slice(&serial));
        let now = to_second(time::OffsetDateTime::now_utc());
        params.not_before = now;
        let expected_not_after = now + time::Duration::days(VALIDITY_DAYS);
        params.not_after = expected_not_after;
        let cert = params.self_signed(&key_pair).expect("self sign");

        let (parsed_serial, parsed_not_after) =
            parse_cert_meta(cert.der()).expect("parse certificate");

        assert_eq!(parsed_serial, hex_lower(&serial));
        // DER time fields hold second precision only, which is why the issue time is truncated.
        assert_eq!(parsed_not_after, format_rfc3339(expected_not_after));
    }

    /// Malformed input must not panic — key-store entries are outside our control and a corrupt
    /// one should degrade to "no certificate" rather than crash the client at startup.
    #[test]
    fn parse_cert_meta_rejects_garbage() {
        assert!(parse_cert_meta(&[]).is_none());
        assert!(parse_cert_meta(&[0x30, 0x03, 0x02, 0x01, 0x01]).is_none());
        assert!(parse_cert_meta(&[0xff; 32]).is_none());
    }

    /// The renewal window keeps a certificate from expiring unnoticed, so check its boundaries
    /// and the unreadable-input fallback rather than only the obvious middle case.
    #[test]
    fn renewal_window_edges() {
        let now = time::OffsetDateTime::now_utc();
        let at = |days: i64| format_rfc3339(now + time::Duration::days(days));

        assert!(!needs_renewal_at(&at(100), now));
        assert!(!needs_renewal_at(&at(RENEW_BEFORE_DAYS + 1), now));
        assert!(needs_renewal_at(&at(RENEW_BEFORE_DAYS - 1), now));
        // Already expired is still a renewal: a machine returning from a long shutdown must
        // re-issue rather than keep reporting a dead certificate.
        assert!(needs_renewal_at(&at(-1), now));
        // Unparseable input must not pin the client to a certificate it cannot judge.
        assert!(needs_renewal_at("not-a-timestamp", now));
    }

    /// Windows hands back `r || s`, and a certificate carrying those bytes verbatim is rejected by
    /// every verifier, so the conversion is pinned with the two awkward cases: a component whose top
    /// bit is set needs a padding byte, and one with leading zeros must lose them.
    #[test]
    fn ecdsa_raw_signature_becomes_der() {
        let mut raw = vec![0u8; 64];
        raw[0] = 0x80; // r starts with a set high bit
        raw[31] = 0x01;
        raw[63] = 0x7f; // s keeps a leading zero byte that DER must strip

        let der = ecdsa_sig_to_der(raw);

        assert_eq!(der[0], 0x30);
        assert_eq!(der[1] as usize, der.len() - 2);
        // r: 33 value bytes, so the INTEGER header carries 0x21 and a padding zero.
        assert_eq!(&der[2..5], &[0x02, 0x21, 0x00]);
        assert_eq!(der[5], 0x80);
        assert_eq!(der[36], 0x01);
        // s: the leading zero is gone, leaving a single byte.
        assert_eq!(&der[37..40], &[0x02, 0x01, 0x7f]);
        assert_eq!(der.len(), 40);
    }

    /// A zero component is legal in a signature and still needs one zero byte, and input that is
    /// already DER must survive untouched instead of being wrapped a second time.
    #[test]
    fn ecdsa_signature_conversion_edges() {
        assert_eq!(
            ecdsa_sig_to_der(vec![0u8; 64]),
            vec![0x30, 0x06, 0x02, 0x01, 0x00, 0x02, 0x01, 0x00]
        );

        let already_der = vec![0x30, 0x06, 0x02, 0x01, 0x01, 0x02, 0x01, 0x02];
        assert_eq!(ecdsa_sig_to_der(already_der.clone()), already_der);
        assert!(ecdsa_sig_to_der(Vec::new()).is_empty());
    }

    /// The reported capability is what the server stores against the device, so the two inputs must
    /// not be interchangeable: an installed certificate decides it, and the platform ceiling is only
    /// the fallback. Reporting the ceiling while a certificate is installed is precisely how an
    /// untrusted machine passes itself off as a working one.
    #[test]
    fn installed_certificate_decides_reported_capability() {
        let info = |capability| CertInfo {
            fingerprint: "00".repeat(32),
            serial: "01".to_string(),
            not_after: "2030-01-01T00:00:00Z".to_string(),
            capability,
            store: "test store",
        };
        let with_cert = |capability| CertStatus {
            capability: CertCapability::Full,
            store: "test store",
            installed: Some(info(capability)),
        };

        // The case this exists for: a certificate the OS does not trust must not be reported as
        // the platform's `full`.
        assert_eq!(
            effective_capability(&with_cert(CertCapability::Partial)),
            CertCapability::Partial
        );
        assert_eq!(
            effective_capability(&with_cert(CertCapability::Full)),
            CertCapability::Full
        );

        // Nothing installed: the ceiling is the answer, which is what keeps a misconfigured device
        // (`full`, no certificate) distinguishable from one that cannot host a certificate.
        assert_eq!(
            effective_capability(&CertStatus {
                capability: CertCapability::Full,
                store: "test store",
                installed: None,
            }),
            CertCapability::Full
        );
        assert_eq!(
            effective_capability(&CertStatus {
                capability: CertCapability::Unavailable,
                store: "none",
                installed: None,
            }),
            CertCapability::Unavailable
        );
    }
}
