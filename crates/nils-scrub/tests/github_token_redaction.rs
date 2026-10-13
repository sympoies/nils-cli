use nils_scrub::{PATTERN_SET, REDACTION_TOKEN, scrub_text};

fn check(input: &str, expected: &str, offset: usize, length: usize) {
    let result = scrub_text(input);
    assert_eq!(result.redacted, expected);
    assert_eq!(PATTERN_SET, "v1");
    assert_eq!(REDACTION_TOKEN, "[REDACTED]");
    assert_eq!(result.matches.len(), usize::from(length > 0));
    if length > 0 {
        let matched = &result.matches[0];
        assert_eq!(matched.pattern_id, "github-token");
        assert_eq!(matched.offset, offset);
        assert_eq!(matched.length, length);
        assert_eq!(matched.redaction_length, REDACTION_TOKEN.len());
    }
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
fn legacy_bare() {
    for prefix in ["gho_", "ghp_", "ghu_", "ghs_", "ghr_"] {
        let token = format!("{prefix}{}", "a".repeat(36));
        check(&token, "[REDACTED]", 0, token.len());
    }
}
case!(
    legacy_url,
    classic_token(),
    "https://x-access-token:",
    "@example.invalid/repo"
);
case!(stateless_bare, stateless(521), "", "");
case!(
    stateless_url,
    stateless(600),
    "https://x-access-token:",
    "@example.invalid/repo"
);
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
fn legacy_bearer() {
    let token = classic_token();
    let input = format!("Bearer {token}");
    check(&input, "Bearer [REDACTED]", 7, token.len());
}

#[test]
fn stateless_bearer() {
    let token = format!("{}-", stateless(520));
    let input = format!("Bearer {token}");
    check(&input, "Bearer [REDACTED]", 7, token.len());
}

#[test]
fn legacy_sentence_punctuation_is_preserved() {
    for prefix in ["gho_", "ghp_", "ghu_", "ghs_", "ghr_"] {
        let token = format!("{prefix}{}", "a".repeat(36));
        for suffix in [".", "-", ". next"] {
            let input = format!("{token}{suffix}");
            let expected = format!("[REDACTED]{suffix}");
            check(&input, &expected, 0, token.len());
        }
    }
}
