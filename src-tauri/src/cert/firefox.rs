//! Firefox profile integration.
//!
//! Firefox is the one supported browser that does not read the platform key store by itself. From
//! version 87 on it can present client certificates held by the OS, but only once
//! `security.osclientcerts.autoload` is enabled.
//!
//! Enabling that preference is necessary but **not sufficient**, which measurement on Firefox 155
//! for macOS settled. With the key store enabled and Firefox's default selection mode still in
//! force, the browser asked for `CN=devops-client` answered with an unrelated certificate from the
//! same keychain: the identity this client installed was never offered. Which identity is picked is
//! governed by a second preference, `security.default_personal_cert`. At its default of
//! `Select Automatically` Firefox does not narrow the key store down to the CA list the server
//! advertises; at `Ask Every Time` it prompts, and a user who picks our certificate is then
//! received by the server as `subject_CN=devops-client`. Both preferences are therefore written
//! below — either one alone leaves Firefox the single browser that fails after a certificate is
//! installed, and that failure is quiet enough to look like a server-side problem.
//!
//! Chromium needs neither preference: it narrows the key store by the server's CA list on its own
//! and presents the same certificate with no profile change at all.
//!
//! Both platforms ship a native `osclientcerts` backend, so the preference points Firefox at a key
//! store that actually holds the certificate: `CurrentUser\My` on Windows, the login keychain on
//! macOS.
//!
//! The preference is written to `user.js` rather than `prefs.js`: Firefox rewrites `prefs.js` when it
//! exits and would drop an entry written from outside, while `user.js` is re-applied on every
//! startup. A user who later resets a profile keeps the setting as a result, which is what we want,
//! since the certificate in the key store is still the one the server expects.
//!
//! Linux is deliberately out of scope. There is no platform key store for Firefox to read there, so
//! the preference would be a no-op and the certificate has to be imported into each profile's own
//! NSS database instead. That is a different mechanism with a different failure mode, and it belongs
//! to Linux's own work item rather than to this file.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// Preference that lets Firefox read client certificates from the platform key store.
const OSCLIENTCERTS_PREF: &str = "security.osclientcerts.autoload";

/// Preference that decides which of those certificates Firefox actually presents.
///
/// Left at Firefox's default (`Select Automatically`) the key store is readable and the certificate
/// is still never offered, so this preference is as load-bearing as [`OSCLIENTCERTS_PREF`].
const DEFAULT_PERSONAL_CERT_PREF: &str = "security.default_personal_cert";

/// Value written for [`DEFAULT_PERSONAL_CERT_PREF`], handing the choice back to the user.
///
/// The other value Firefox documents, `Select Automatically`, was measured to send an unrelated
/// certificate; no value makes the browser narrow the candidates by the server's CA list.
const ASK_EVERY_TIME: &str = "Ask Every Time";

/// Start of the block this client owns inside `user.js`.
const MARKER_BEGIN: &str = "// >>> devops-client device certificate";

/// End of the block this client owns inside `user.js`.
const MARKER_END: &str = "// <<< devops-client device certificate";

/// How many Firefox profiles were found, and how they responded to being updated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ProfileUpdate {
    /// Profiles whose directory exists, i.e. the profiles Firefox would actually load.
    pub profiles: usize,
    /// Profiles whose `user.js` this call rewrote.
    pub updated: usize,
    /// Profiles that could not be written, for instance a snap-confined directory.
    pub failed: usize,
}

/// Root directory that holds Firefox's `profiles.ini` and its profile folders.
///
/// `dirs::config_dir()` already resolves to the right place on Windows (`%APPDATA%`) and macOS
/// (`~/Library/Application Support`), but on Linux Firefox keeps its state under `~/.mozilla`
/// instead of the XDG config directory, so that one case is spelled out.
fn profile_root() -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        return dirs::home_dir().map(|home| home.join(".mozilla").join("firefox"));
    }
    #[cfg(not(target_os = "linux"))]
    {
        dirs::config_dir().map(|dir| dir.join("Mozilla").join("Firefox"))
    }
}

/// Profile directories Firefox would use, in the order `profiles.ini` lists them.
///
/// Parsed and filtered rather than guessed. Firefox names profile folders with a random prefix
/// (`a1b2c3d4.default-release`), so a hardcoded path would point at the wrong profile or at nothing,
/// and `profiles.ini` keeps entries for profiles the user already deleted.
pub fn profile_dirs() -> Vec<PathBuf> {
    let Some(root) = profile_root() else {
        return Vec::new();
    };
    let ini = std::fs::read_to_string(root.join("profiles.ini")).unwrap_or_default();
    parse_profiles_ini(&ini, &root)
        .into_iter()
        .filter(|dir| dir.is_dir())
        .collect()
}

/// Extracts profile paths from the contents of `profiles.ini`.
///
/// Only `Path` keys are read. `IsRelative=0` marks an absolute path, anything else is joined onto
/// `root`. Sections without a `Path` are skipped, which covers `[General]` and the `[Install*]`
/// sections whose `Default` key is not a profile. Duplicates are dropped because a profile can be
/// listed under more than one section.
fn parse_profiles_ini(ini: &str, root: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut current: Option<(Option<String>, bool)> = None;

    // A section's keys all sit before the next header, so each header closes the one it follows.
    let finish = |section: Option<(Option<String>, bool)>, dirs: &mut Vec<PathBuf>| {
        if let Some((Some(path), relative)) = section {
            let dir = if relative {
                root.join(path)
            } else {
                PathBuf::from(path)
            };
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
    };

    for line in ini.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }

        if line.starts_with('[') {
            finish(current.take(), &mut dirs);
            current = Some((None, true));
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if let Some(section) = current.as_mut() {
            match key.trim() {
                "Path" => section.0 = Some(value.trim().to_string()),
                "IsRelative" => section.1 = value.trim() != "0",
                _ => {}
            }
        }
    }
    finish(current.take(), &mut dirs);

    dirs
}

/// The block this client owns, ending with a newline so appending to a file that lacked one still
/// leaves clean lines behind.
///
/// Both preferences sit inside the one block so [`merge_user_js`] can compare and replace it as a
/// unit, which is also what repairs a profile still carrying the older key-store-only block.
fn managed_block() -> String {
    format!(
        "{}\nuser_pref(\"{}\", true);\nuser_pref(\"{}\", \"{}\");\n{}\n",
        MARKER_BEGIN, OSCLIENTCERTS_PREF, DEFAULT_PERSONAL_CERT_PREF, ASK_EVERY_TIME, MARKER_END
    )
}

/// Merges this client's block into the contents of a `user.js`.
///
/// Only the marked block is touched, so a profile's own preferences survive untouched. Comparing
/// before writing is what makes a repeated call cheap: the profile is left alone once it is correct.
///
/// @return `None` when the content already carries exactly our block and no write is needed,
///         otherwise the complete new file content
fn merge_user_js(existing: &str) -> Option<String> {
    let block = managed_block();

    if let Some(start) = existing.find(MARKER_BEGIN) {
        let end = existing[start..]
            .find(MARKER_END)
            .map(|offset| start + offset + MARKER_END.len());

        // An unterminated block means something else rewrote the file. Replacing everything from our
        // marker to the end is the only safe reading, since a half-written preference would be
        // applied by Firefox exactly as it stands.
        let found = &existing[start..end.unwrap_or(existing.len())];
        if found.trim_end() == block.trim_end() {
            return None;
        }

        let mut merged = String::with_capacity(existing.len() + block.len());
        merged.push_str(&existing[..start]);
        merged.push_str(&block);
        if let Some(end) = end {
            merged.push_str(&existing[end..]);
        }
        return Some(merged);
    }

    let mut merged = String::with_capacity(existing.len() + block.len() + 1);
    merged.push_str(existing);
    if !merged.is_empty() && !merged.ends_with('\n') {
        merged.push('\n');
    }
    merged.push_str(&block);
    Some(merged)
}

/// Brings one profile's `user.js` up to date.
///
/// Failures are returned rather than swallowed so the caller can count them, but they are never
/// fatal: one unwritable profile must not stop the others from being fixed.
///
/// @return `true` when the file was written, `false` when it was already correct
fn sync_user_js(profile: &Path) -> Result<bool, std::io::Error> {
    let user_js = profile.join("user.js");
    let existing = match std::fs::read_to_string(&user_js) {
        Ok(content) => content,
        // A profile without `user.js` is the normal case, not a failure.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };

    match merge_user_js(&existing) {
        Some(content) => {
            std::fs::write(&user_js, content)?;
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Enables the platform key store for Firefox in every profile it can find.
///
/// Only meaningful where Firefox implements `osclientcerts` against a real key store: Windows reads
/// `CurrentUser\My` and macOS reads the login keychain, both of which are where the certificate is
/// installed. Callers on platforms without such a store — Linux today — should not call this at
/// all, because the preference would leave Firefox looking in a place that holds nothing.
pub fn enable_os_client_certs() -> ProfileUpdate {
    let dirs = profile_dirs();
    let mut updated = 0;
    let mut failed = 0;

    for dir in &dirs {
        match sync_user_js(dir) {
            Ok(true) => updated += 1,
            Ok(false) => {}
            Err(_) => failed += 1,
        }
    }

    ProfileUpdate {
        profiles: dirs.len(),
        updated,
        failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real `profiles.ini` mixes relative paths, an absolute one and sections that hold no path at
    /// all. Only the profiles may come out, and in file order.
    #[test]
    fn profiles_ini_reads_both_path_forms() {
        let ini = "\
[General]
StartWithLastProfile=1
Version=2

[Profile0]
Name=default
IsRelative=1
Path=Profiles/a1b2c3d4.default-release
Default=1

[Install4F96D1932A9F858E]
Default=Profiles/a1b2c3d4.default-release
Locked=1

[Profile1]
Name=work
IsRelative=0
Path=/mnt/data/firefox-work
";
        let root = Path::new("/home/dev");
        let dirs = parse_profiles_ini(ini, root);

        assert_eq!(
            dirs,
            vec![
                root.join("Profiles/a1b2c3d4.default-release"),
                PathBuf::from("/mnt/data/firefox-work"),
            ]
        );
    }

    /// `IsRelative` is absent in hand-written files and `[Install*]` sections carry a `Default=`
    /// that looks like a path, so an unset flag must mean relative and a foreign key must be ignored.
    #[test]
    fn profiles_ini_defaults_to_relative_paths() {
        let ini = "[Profile0]\nPath=abc.default\n";
        assert_eq!(
            parse_profiles_ini(ini, Path::new("/root")),
            vec![PathBuf::from("/root/abc.default")]
        );
    }

    /// A profile can be listed twice after a Firefox profile-manager edit; writing the same `user.js`
    /// twice is harmless but reporting it twice would mislead the caller.
    #[test]
    fn profiles_ini_drops_duplicates() {
        let ini = "[Profile0]\nPath=x.default\n[Profile1]\nPath=x.default\n";
        assert_eq!(parse_profiles_ini(ini, Path::new("/root")).len(), 1);
    }

    /// The whole point of the block check: a profile that is already correct must not be rewritten,
    /// otherwise every launch would touch dozens of files for nothing.
    #[test]
    fn user_js_write_is_idempotent() {
        let written = merge_user_js("").expect("empty file needs the block");
        assert!(written.contains(OSCLIENTCERTS_PREF));
        assert_eq!(merge_user_js(&written), None);
    }

    /// Users keep their own preferences in `user.js`; losing them would make this client hostile.
    /// An existing `false` value is the one case that must be corrected rather than preserved.
    #[test]
    fn user_js_preserves_foreign_prefs_and_fixes_disabled_value() {
        let existing = "\
user_pref(\"browser.tabs.warnOnClose\", false);
// >>> devops-client device certificate
user_pref(\"security.osclientcerts.autoload\", false);
// <<< devops-client device certificate
user_pref(\"toolkit.telemetry.enabled\", false);
";
        let merged = merge_user_js(existing).expect("the disabled value must be corrected");

        assert!(merged.contains("browser.tabs.warnOnClose"));
        assert!(merged.contains("toolkit.telemetry.enabled"));
        assert!(merged.contains("user_pref(\"security.osclientcerts.autoload\", true);"));
        assert_eq!(merged.matches(MARKER_BEGIN).count(), 1);
        assert_eq!(merged.matches(OSCLIENTCERTS_PREF).count(), 1);
        assert_eq!(merged.matches(DEFAULT_PERSONAL_CERT_PREF).count(), 1);
    }

    /// A file that lost our closing marker (truncated, or merged by a sync tool) must still end up
    /// with exactly one usable block instead of a stale preference that Firefox would apply as-is.
    #[test]
    fn user_js_replaces_unterminated_block() {
        let existing = "user_pref(\"a.b\", 1);\n// >>> devops-client device certificate\nuser_pref(\"security.osclientcerts.autoload\", false);\n";
        let merged = merge_user_js(existing).expect("the truncated block must be repaired");

        assert_eq!(merged.matches(MARKER_BEGIN).count(), 1);
        assert!(merged.ends_with(&format!("{}\n", MARKER_END)));
        assert!(merged.contains("user_pref(\"a.b\", 1);"));
    }

    /// A file whose last line has no newline must not end up with a preference glued onto it.
    #[test]
    fn user_js_appends_after_unterminated_line() {
        let merged =
            merge_user_js("user_pref(\"a.b\", 1);").expect("a missing block must be added");
        assert!(merged.starts_with("user_pref(\"a.b\", 1);\n// >>>"));
    }

    /// Both preferences are what Firefox matches on, and the block is asserted against the exact
    /// literals rather than only against our own constants: a rename would keep the key store
    /// enabled while the certificate is still never offered, which is the failure this file exists
    /// to prevent. The selection preference in particular was missing here once, and the tests
    /// passed while Firefox sent an unrelated certificate.
    #[test]
    fn managed_block_matches_firefox_expected_syntax() {
        let block = managed_block();
        assert!(block.starts_with(MARKER_BEGIN));
        assert!(block.trim_end().ends_with(MARKER_END));
        assert!(block.contains("user_pref(\"security.osclientcerts.autoload\", true);"));
        assert!(
            block.contains("user_pref(\"security.default_personal_cert\", \"Ask Every Time\");")
        );
        assert_eq!(OSCLIENTCERTS_PREF, "security.osclientcerts.autoload");
        assert_eq!(DEFAULT_PERSONAL_CERT_PREF, "security.default_personal_cert");
        assert_eq!(ASK_EVERY_TIME, "Ask Every Time");
    }

    /// The block is rewritten as a unit, so a profile carrying the older key-store-only block — or
    /// one where the user turned selection back to automatic — must gain the second preference
    /// without ending up with either preference listed twice.
    #[test]
    fn managed_block_upgrades_older_profile() {
        let existing = "\
// >>> devops-client device certificate
user_pref(\"security.osclientcerts.autoload\", true);
// <<< devops-client device certificate
";
        let merged = merge_user_js(existing).expect("the older block must be upgraded");
        assert_eq!(merged.matches(MARKER_BEGIN).count(), 1);
        assert_eq!(merged.matches(DEFAULT_PERSONAL_CERT_PREF).count(), 1);
        assert!(
            merged.contains("user_pref(\"security.default_personal_cert\", \"Ask Every Time\");")
        );
    }
}
