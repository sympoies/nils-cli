//! Provider-neutral management of named accounts stored as `<name>.json`.
//!
//! Provider CLIs keep one stored secret per account in a secret directory and
//! switch, save, remove, and report them through the same `auth` commands.
//! This module owns the parts of that surface that must not drift between
//! providers: target validation and `.json` normalization, resolution by name
//! or email, the overwrite and removal confirmation flow, the exit codes, and
//! the JSON-or-stderr failure report. What a stored secret contains, and how it
//! is read, written, or projected as the active login, stays with the provider
//! behind [`AccountStore`].

use std::io::{self, BufRead, IsTerminal, Write};

use serde_json::Value;

use crate::diag_output;

/// A runtime failure, a missing account, or a declined confirmation.
pub const EXIT_FAILED: i32 = 1;
/// The target matched several accounts, or the active login matched none.
pub const EXIT_UNMATCHED: i32 = 2;
/// Invalid target name or missing required confirmation for removal.
pub const EXIT_USAGE: i32 = 64;

const ACCOUNT_FILE_SUFFIX: &str = ".json";

/// Longest account nickname, in bytes.
pub const MAX_ACCOUNT_NICKNAME_BYTES: usize = 64;

/// Whether `nickname` is a safe account nickname:
/// `^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$`.
///
/// Nicknames name stored secrets and travel as broker and CLI arguments, so
/// the leading alphanumeric byte keeps them from reading as an option
/// (`--format`), a hidden file, or a dot path segment (`.`, `..`).
pub fn is_valid_account_nickname(nickname: &str) -> bool {
    nickname.len() <= MAX_ACCOUNT_NICKNAME_BYTES
        && nickname
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && nickname
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// Whether `target` could name a path outside the secret directory.
pub fn is_invalid_account_target(target: &str) -> bool {
    target.contains('/') || target.contains('\\') || target.contains("..")
}

/// The stored file name for `target`: `name` and `name.json` both map to
/// `name.json`.
pub fn account_file_name(target: &str) -> String {
    if target.ends_with(ACCOUNT_FILE_SUFFIX) {
        return target.to_string();
    }
    format!("{target}{ACCOUNT_FILE_SUFFIX}")
}

/// The account name for `target`, without a trailing `.json`.
pub fn account_name(target: &str) -> &str {
    target.strip_suffix(ACCOUNT_FILE_SUFFIX).unwrap_or(target)
}

/// Provider access to the stored accounts, addressed by file name
/// (`<name>.json`).
pub trait AccountStore {
    /// File names of every stored account, in any order.
    fn account_files(&self) -> Vec<String>;

    /// Whether `file_name` is a stored account.
    fn has_account(&self, file_name: &str) -> bool;

    /// The email address the stored account belongs to, when it records one.
    fn account_email(&self, file_name: &str) -> Option<String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountResolution {
    /// The single matching account file name.
    Exact(String),
    /// Every matching account file name, sorted.
    Ambiguous {
        candidates: Vec<String>,
    },
    NotFound,
}

/// Resolve a validated `target` to one stored account.
///
/// A bare name or `name.json` matches that file. A target containing `@` is
/// first tried as a literal file name. Otherwise the target is compared,
/// case-insensitively, with each account's full email address (targets with
/// `@`) or email local part (targets without).
pub fn resolve_account<S: AccountStore + ?Sized>(store: &S, target: &str) -> AccountResolution {
    let direct = if target.contains('@') {
        target.to_string()
    } else {
        account_file_name(target)
    };
    if store.has_account(&direct) {
        return AccountResolution::Exact(direct);
    }
    resolve_account_by_email(store, target)
}

/// Resolve `target` by full email address or email local part only.
pub fn resolve_account_by_email<S: AccountStore + ?Sized>(
    store: &S,
    target: &str,
) -> AccountResolution {
    let query = target.to_lowercase();
    let want_full = target.contains('@');
    let mut matches: Vec<String> = store
        .account_files()
        .into_iter()
        .filter(|file_name| {
            let Some(email) = store.account_email(file_name) else {
                return false;
            };
            let email = email.to_lowercase();
            if want_full {
                email == query
            } else {
                email.split('@').next() == Some(query.as_str())
            }
        })
        .collect();
    matches.sort();

    match matches.len() {
        0 => AccountResolution::NotFound,
        1 => AccountResolution::Exact(matches.remove(0)),
        _ => AccountResolution::Ambiguous {
            candidates: matches,
        },
    }
}

/// A destructive account change that needs confirmation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmAction {
    /// Replacing a stored account.
    Overwrite,
    /// Deleting a stored account.
    Remove,
}

impl ConfirmAction {
    /// The error code reported when confirmation is required but unavailable.
    pub fn required_error_code(self) -> &'static str {
        match self {
            Self::Overwrite => "overwrite-confirmation-required",
            Self::Remove => "usage-error",
        }
    }

    /// The exit code when confirmation is required but unavailable.
    pub fn required_exit_code(self) -> i32 {
        match self {
            Self::Overwrite => EXIT_FAILED,
            Self::Remove => EXIT_USAGE,
        }
    }

    /// The exit code when the prompt was answered with anything but yes.
    pub fn declined_exit_code(self) -> i32 {
        EXIT_FAILED
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirmation {
    /// `--yes` was given or the prompt was answered yes.
    Confirmed,
    /// The prompt was answered with anything but yes.
    Declined,
    /// JSON output or non-interactive stdio without `--yes`.
    Required,
}

/// Confirm a destructive change. `yes` confirms; JSON output or stdio that is
/// not a terminal requires `yes`; otherwise `<prompt> [y/N]: ` is asked on
/// stderr and read from stdin.
pub fn confirm(yes: bool, output_json: bool, prompt: &str) -> io::Result<Confirmation> {
    let interactive = io::stdin().is_terminal() && io::stdout().is_terminal();
    confirm_with(
        yes,
        output_json,
        interactive,
        prompt,
        &mut io::stdin().lock(),
        &mut io::stderr(),
    )
}

/// [`confirm`] with explicit terminal state and streams.
pub fn confirm_with(
    yes: bool,
    output_json: bool,
    interactive: bool,
    prompt: &str,
    input: &mut dyn BufRead,
    prompt_out: &mut dyn Write,
) -> io::Result<Confirmation> {
    if yes {
        return Ok(Confirmation::Confirmed);
    }
    if output_json || !interactive {
        return Ok(Confirmation::Required);
    }
    write!(prompt_out, "{prompt} [y/N]: ")?;
    prompt_out.flush()?;
    let mut line = String::new();
    input.read_line(&mut line)?;
    let answer = line.trim().to_ascii_lowercase();
    Ok(if matches!(answer.as_str(), "y" | "yes") {
        Confirmation::Confirmed
    } else {
        Confirmation::Declined
    })
}

/// Where an account command reports its result.
#[derive(Debug, Clone, Copy)]
pub struct AccountOutput<'a> {
    pub schema_version: &'a str,
    pub command: &'a str,
    pub output_json: bool,
}

impl AccountOutput<'_> {
    /// Report a failure as a JSON error envelope on stdout, or as `message` on
    /// stderr, and return `exit_code`.
    pub fn fail(
        &self,
        code: &str,
        message: impl Into<String>,
        details: Option<Value>,
        exit_code: i32,
    ) -> anyhow::Result<i32> {
        let message = message.into();
        if self.output_json {
            diag_output::emit_error(self.schema_version, self.command, code, message, details)?;
        } else {
            eprintln!("{message}");
        }
        Ok(exit_code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::collections::BTreeMap;

    struct MemoryStore(BTreeMap<&'static str, Option<&'static str>>);

    impl AccountStore for MemoryStore {
        fn account_files(&self) -> Vec<String> {
            self.0.keys().rev().map(|name| name.to_string()).collect()
        }

        fn has_account(&self, file_name: &str) -> bool {
            self.0.contains_key(file_name)
        }

        fn account_email(&self, file_name: &str) -> Option<String> {
            self.0.get(file_name).copied().flatten().map(str::to_string)
        }
    }

    fn store() -> MemoryStore {
        MemoryStore(BTreeMap::from([
            ("alpha.json", Some("Alpha@example.com")),
            ("beta-1.json", Some("beta@example.com")),
            ("beta-2.json", Some("beta@example.com")),
            ("plain.json", None),
            ("team@example.com", Some("other@example.com")),
        ]))
    }

    #[test]
    fn targets_reject_paths_and_normalize_the_json_suffix() {
        assert!(is_invalid_account_target("../a.json"));
        assert!(is_invalid_account_target("a/b"));
        assert!(is_invalid_account_target(r"a\b"));
        assert!(!is_invalid_account_target("alpha"));
        assert!(!is_invalid_account_target("alpha@example.com"));
        assert_eq!(account_file_name("alpha"), "alpha.json");
        assert_eq!(account_file_name("alpha.json"), "alpha.json");
        assert_eq!(account_name("alpha.json"), "alpha");
        assert_eq!(account_name("alpha"), "alpha");
    }

    #[test]
    fn account_nicknames_follow_one_shared_rule() {
        for valid in ["a", "alpha.team-1_x", "Team2", &"a".repeat(64)] {
            assert!(is_valid_account_nickname(valid), "{valid}");
        }
        for invalid in [
            "",
            "-x",
            "--format",
            ".",
            "..",
            ".hidden",
            "_hidden",
            "../up",
            "a/b",
            "has space",
            "semi;colon",
            "person@example.com",
            &"a".repeat(65),
        ] {
            assert!(!is_valid_account_nickname(invalid), "{invalid}");
        }
    }

    #[test]
    fn resolve_account_matches_name_file_email_and_local_part() {
        let store = store();
        for target in ["alpha", "alpha.json", "alpha@example.com", "ALPHA"] {
            assert_eq!(
                resolve_account(&store, target),
                AccountResolution::Exact("alpha.json".to_string()),
                "{target}"
            );
        }
        assert_eq!(
            resolve_account(&store, "plain"),
            AccountResolution::Exact("plain.json".to_string())
        );
        // A target with `@` is tried as a literal file name first.
        assert_eq!(
            resolve_account(&store, "team@example.com"),
            AccountResolution::Exact("team@example.com".to_string())
        );
    }

    #[test]
    fn resolve_account_reports_ambiguous_and_missing_targets() {
        let store = store();
        let ambiguous = AccountResolution::Ambiguous {
            candidates: vec!["beta-1.json".to_string(), "beta-2.json".to_string()],
        };
        assert_eq!(resolve_account(&store, "beta"), ambiguous);
        assert_eq!(resolve_account(&store, "beta@example.com"), ambiguous);
        assert_eq!(
            resolve_account(&store, "missing"),
            AccountResolution::NotFound
        );
        assert_eq!(
            resolve_account(&store, "alpha.json@example.com"),
            AccountResolution::NotFound
        );
    }

    #[test]
    fn confirm_requires_yes_without_an_interactive_terminal_or_with_json() {
        let mut out = Vec::new();
        let mut input = io::Cursor::new(b"y\n".to_vec());
        let answer =
            |yes, json, interactive, input: &mut io::Cursor<Vec<u8>>, out: &mut Vec<u8>| {
                confirm_with(yes, json, interactive, "remove?", input, out).expect("confirm")
            };
        assert_eq!(
            answer(true, true, false, &mut input, &mut out),
            Confirmation::Confirmed
        );
        assert_eq!(
            answer(false, true, true, &mut input, &mut out),
            Confirmation::Required
        );
        assert_eq!(
            answer(false, false, false, &mut input, &mut out),
            Confirmation::Required
        );
        assert!(out.is_empty());
    }

    #[test]
    fn confirm_prompts_and_accepts_only_yes() {
        for (reply, expected) in [
            ("y\n", Confirmation::Confirmed),
            (" YES \n", Confirmation::Confirmed),
            ("n\n", Confirmation::Declined),
            ("", Confirmation::Declined),
        ] {
            let mut out = Vec::new();
            let mut input = io::Cursor::new(reply.as_bytes().to_vec());
            let answer =
                confirm_with(false, false, true, "overwrite?", &mut input, &mut out).expect("ok");
            assert_eq!(answer, expected, "{reply:?}");
            assert_eq!(String::from_utf8(out).expect("utf8"), "overwrite? [y/N]: ");
        }
    }

    #[test]
    fn confirm_actions_keep_their_error_and_exit_codes() {
        assert_eq!(
            ConfirmAction::Overwrite.required_error_code(),
            "overwrite-confirmation-required"
        );
        assert_eq!(ConfirmAction::Overwrite.required_exit_code(), 1);
        assert_eq!(ConfirmAction::Remove.required_error_code(), "usage-error");
        assert_eq!(ConfirmAction::Remove.required_exit_code(), 64);
        assert_eq!(ConfirmAction::Remove.declined_exit_code(), 1);
        assert_eq!(ConfirmAction::Overwrite.declined_exit_code(), 1);
    }
}
