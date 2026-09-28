//! Linux backend: no key store yet, reported honestly as `unavailable`.
//!
//! Linux is the one platform without an OS-wide client certificate store that browsers read on
//! their own: Chrome looks in `~/.pki/nssdb` and Firefox in each profile's own NSS database, so a
//! single key cannot serve every browser the way the Windows store and the macOS login keychain do.
//! Making Linux work means writing into those NSS databases directly (PKCS#11 / softoken), which is
//! a mechanism of its own rather than a variation of the other two.
//!
//! Until that lands, this backend deliberately installs nothing and reports
//! [`CertCapability::Unavailable`]. That is not a stub to be filled in later without consequence —
//! it is the value the server acts on: a device reporting `unavailable` keeps the legacy presence
//! checks instead of being treated as a device whose certificate went missing, so Linux users see
//! no change at all rather than an authentication failure. Returning `Full` here would make every
//! Linux machine look misconfigured; returning a certificate we cannot keep in a real store would
//! be worse, since the browser would never present it and the server would reject the session.
//!
//! The module exists — and is compiled — on every Linux build so that the platform stays reachable:
//! dropping it would only move the failure from a clear `unavailable` at runtime to a compile error
//! in CI, or worse, to a silent fallback in [`super::provider`].

use super::{CertCapability, CertError, CertInfo, DeviceCertProvider};

/// Reported to the UI and the heartbeat while no Linux key store exists.
///
/// Worded as a store rather than a capability so the device panel reads the same way on every
/// platform; the capability field next to it already says `unavailable`.
const STORE: &str = "none (NSS import not implemented)";

/// Placeholder provider for Linux.
///
/// Implements the full trait so the platform dispatcher stays uniform, but every operation either
/// reports "cannot" or does nothing. See the module docs for why reporting `unavailable` is the
/// correct behaviour rather than a missing feature.
pub struct NssProvider;

impl DeviceCertProvider for NssProvider {
    fn store_name(&self) -> &'static str {
        STORE
    }

    /// Always `unavailable`: there is no store here to hold a key, so no machine running Linux can
    /// present a client certificate regardless of its configuration.
    fn capability(&self) -> CertCapability {
        CertCapability::Unavailable
    }

    /// Refuses rather than installing into a store that browsers do not read.
    ///
    /// The error is [`CertError::Unsupported`] and not `Platform` on purpose: the caller logs the
    /// two differently, and this is a platform limitation the user cannot fix by unlocking
    /// anything, not a key store that refused an operation.
    fn ensure(&self, _owner: &str) -> Result<CertInfo, CertError> {
        Err(CertError::Unsupported(
            "client certificates are not implemented on Linux yet".to_string(),
        ))
    }

    /// No certificate can be installed, so none can be found.
    fn status(&self) -> Option<CertInfo> {
        None
    }

    /// Succeeds without doing anything: there is never a certificate here to remove, and reporting
    /// a failure would make sign-out look broken on a platform that has nothing to clean up.
    fn remove(&self) -> Result<(), CertError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The heartbeat sends this value to the server, where it decides whether a missing certificate
    /// is expected or a misconfiguration. A Linux machine must never report anything else.
    #[test]
    fn capability_is_unavailable() {
        assert_eq!(NssProvider.capability(), CertCapability::Unavailable);
    }

    /// Installing must fail loudly instead of pretending to succeed: a caller that got `Ok` would
    /// go on to report a fingerprint to the server for a certificate no browser can present.
    #[test]
    fn ensure_reports_unsupported() {
        assert!(matches!(
            NssProvider.ensure("dev@host"),
            Err(CertError::Unsupported(_))
        ));
    }

    /// `status` is consulted on every launch and at sign-out; both must treat Linux as "nothing
    /// installed" rather than as an error, or the client would look broken on that platform.
    #[test]
    fn status_and_remove_are_no_ops() {
        assert!(NssProvider.status().is_none());
        assert!(NssProvider.remove().is_ok());
    }
}
