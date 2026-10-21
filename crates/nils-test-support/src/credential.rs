//! Assertions over credential-bearing text that never print the text.
//!
//! `assert_eq!` on two strings prints both sides when they differ, so a mismatch
//! between a fixture and a real credential file puts the credential in the test
//! log. These helpers fail with a label, a length, and a fingerprint instead
//! (sympoies/nils-cli#2755).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Length and a fixed-key hash of `text`: enough to tell two values apart in a
/// failure message without revealing either.
pub fn fingerprint(text: &str) -> String {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    format!("len={} fp={:016x}", text.len(), hasher.finish())
}

/// Assert two credential-bearing strings are equal without printing them.
#[track_caller]
pub fn assert_credential_eq(label: &str, actual: &str, expected: &str) {
    if actual != expected {
        panic!(
            "{label}: credential text differs (actual {}, expected {}); contents withheld",
            fingerprint(actual),
            fingerprint(expected)
        );
    }
}

/// Assert `haystack` does not contain `secret`, without printing either.
#[track_caller]
pub fn assert_credential_absent(label: &str, haystack: &str, secret: &str) {
    if haystack.contains(secret) {
        panic!(
            "{label}: output contains credential text ({}); contents withheld",
            fingerprint(secret)
        );
    }
}

/// Assert that the file at `path` holds exactly `expected`, without printing
/// either side.
#[track_caller]
pub fn assert_credential_file_eq(label: &str, path: &std::path::Path, expected: &str) {
    let actual = std::fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("{label}: read {}: {err}", path.display()));
    assert_credential_eq(label, &actual, expected);
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::panic::catch_unwind;

    const CANARY: &str = "canary-credential-value-0123456789";

    fn panic_text(run: impl FnOnce() + std::panic::UnwindSafe) -> String {
        let payload = catch_unwind(run).expect_err("must panic");
        payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|text| text.to_string()))
            .unwrap_or_default()
    }

    #[test]
    fn equal_text_passes_and_fingerprints_distinguish_values() {
        assert_credential_eq("same", CANARY, CANARY);
        assert_credential_absent("absent", "harmless output", CANARY);
        assert_eq!(fingerprint(CANARY), fingerprint(CANARY));
        assert!(fingerprint(CANARY) != fingerprint("another-value"));
    }

    #[test]
    fn a_mismatch_reports_fingerprints_but_not_the_text() {
        let message = panic_text(|| assert_credential_eq("auth file", CANARY, "other-value"));
        assert!(message.contains("auth file"), "{message}");
        assert!(message.contains("fp="), "{message}");
        assert!(!message.contains(CANARY), "{message}");
        assert!(!message.contains("other-value"), "{message}");
    }

    #[test]
    fn leaked_text_is_reported_without_printing_it() {
        let message = panic_text(|| {
            assert_credential_absent("stdout", &format!("log line {CANARY} end"), CANARY)
        });
        assert!(message.contains("stdout"), "{message}");
        assert!(!message.contains(CANARY), "{message}");
    }

    #[test]
    fn a_file_mismatch_withholds_the_contents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        std::fs::write(&path, CANARY).unwrap();
        assert_credential_file_eq("auth", &path, CANARY);
        let message = panic_text(|| assert_credential_file_eq("auth", &path, "different"));
        assert!(!message.contains(CANARY), "{message}");
    }
}
