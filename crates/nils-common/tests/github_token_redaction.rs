use nils_common::redact::redact_text;
use pretty_assertions::assert_eq;

fn check(input: &str, expected: &str, _offset: usize, length: usize) {
    let result = redact_text(input);
    assert_eq!(result.value, expected);
    assert_eq!(result.replacements, usize::from(length > 0));
}

// Deliberately synthetic; token lengths include the four-byte prefix.
fn classic_token() -> String {
    format!("ghs_{}", "a".repeat(36))
}

fn stateless(length: usize) -> String {
    let prefix = "ghs_123456_";
    let body = "Ab9_-".repeat(length);
    format!(
        "{prefix}{}.{}.{}",
        &body[..12],
        &body[..12],
        &body[..length - prefix.len() - 26]
    )
}

macro_rules! case {
    ($name:ident, $token:expr, $before:expr, $after:expr) => {
        #[test]
        fn $name() {
            let token = $token;
            let input = format!("{}{}{}", $before, token, $after);
            let expected = format!("{}[REDACTED]{}", $before, $after);
            check(&input, &expected, $before.len(), token.len());
        }
    };
}

#[test]
fn classic_bare() {
    for prefix in ["gho_", "ghp_", "ghu_", "ghs_", "ghr_"] {
        let token = format!("{prefix}{}", "a".repeat(36));
        check(&token, "[REDACTED]", 0, token.len());
    }
}
#[test]
fn classic_url() {
    let token = classic_token();
    let input = format!("https://x-access-token:{token}@example.invalid/repo");
    // The assignment redactor already removes the entire URL credential value.
    check(&input, "https://x-access-token:[REDACTED]", 0, token.len());
}
case!(stateless_bare, stateless(521), "", "");
#[test]
fn stateless_url() {
    let token = stateless(600);
    let input = format!("https://x-access-token:{token}@example.invalid/repo");
    // The assignment redactor already removes the entire URL credential value.
    check(&input, "https://x-access-token:[REDACTED]", 0, token.len());
}
case!(
    stateless_cutoff,
    stateless(521)[..128].to_string(),
    "prefix ",
    ""
);
case!(
    stateless_hyphen_tail,
    format!("{}-", stateless(127)),
    "",
    "\nnext"
);
case!(
    stateless_dot_tail,
    format!("{}.", stateless(599)),
    "",
    "\" next"
);
case!(
    stateless_underscore_tail,
    format!("{}_", stateless(39)),
    "",
    " next"
);
case!(stateless_assignment, stateless(521), "token=", " next");

#[test]
fn ordinary_dotted_identifiers_are_unchanged() {
    let input =
        "service.component-v2_name ghz_abcdefghijklmnopqrstuvwxyz0123456789 ghs_short.part-name";
    check(input, input, 0, 0);
}

#[test]
fn classic_bearer() {
    let token = classic_token();
    let input = format!("Bearer {token}");
    check(&input, "[REDACTED]", 0, input.len());
}

#[test]
fn stateless_bearer() {
    let token = format!("{}-", stateless(520));
    let input = format!("Bearer {token}");
    check(&input, "[REDACTED]", 0, input.len());
}

#[test]
fn classic_sentence_punctuation_is_preserved() {
    for prefix in ["gho_", "ghp_", "ghu_", "ghs_", "ghr_"] {
        let token = format!("{prefix}{}", "a".repeat(36));
        for suffix in [".", "-", ". next"] {
            let input = format!("{token}{suffix}");
            let expected = format!("[REDACTED]{suffix}");
            check(&input, &expected, 0, token.len());
        }
    }
}
