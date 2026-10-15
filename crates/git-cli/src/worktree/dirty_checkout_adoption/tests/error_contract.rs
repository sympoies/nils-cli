use super::*;

#[test]
fn dirty_checkout_error_contract_exhaustively_freezes_codes_exits_and_json_envelopes() {
    fn expected(kind: DirtyCheckoutErrorKind) -> (&'static str, i32) {
        match kind {
            DirtyCheckoutErrorKind::CleanCheckout => ("dirty-checkout-clean", exit::DATA),
            DirtyCheckoutErrorKind::UnsupportedGitState => {
                ("dirty-checkout-unsupported-git-state", exit::DATA)
            }
            DirtyCheckoutErrorKind::ChallengeExpired => {
                ("dirty-checkout-challenge-expired", exit::DATA)
            }
            DirtyCheckoutErrorKind::ChallengeReused => {
                ("dirty-checkout-challenge-reused", exit::DATA)
            }
            DirtyCheckoutErrorKind::ChallengeDrift => {
                ("dirty-checkout-challenge-drift", exit::DATA)
            }
            DirtyCheckoutErrorKind::ForeignLease => ("dirty-checkout-foreign-lease", exit::DATA),
            DirtyCheckoutErrorKind::MalformedState => {
                ("dirty-checkout-malformed-state", exit::DATA)
            }
            DirtyCheckoutErrorKind::InvalidInput => ("dirty-checkout-invalid-input", exit::DATA),
            DirtyCheckoutErrorKind::Timeout => ("dirty-checkout-timeout", exit::RUNTIME),
            DirtyCheckoutErrorKind::ResourceUnavailable => {
                ("dirty-checkout-resource-unavailable", exit::RUNTIME)
            }
            DirtyCheckoutErrorKind::SnapshotFailed => {
                ("dirty-checkout-snapshot-failed", exit::RUNTIME)
            }
            DirtyCheckoutErrorKind::AdoptionFailed => {
                ("dirty-checkout-adoption-failed", exit::RUNTIME)
            }
            DirtyCheckoutErrorKind::RevocationFailed => {
                ("dirty-checkout-revoke-failed", exit::RUNTIME)
            }
        }
    }

    let kinds = [
        DirtyCheckoutErrorKind::CleanCheckout,
        DirtyCheckoutErrorKind::UnsupportedGitState,
        DirtyCheckoutErrorKind::ChallengeExpired,
        DirtyCheckoutErrorKind::ChallengeReused,
        DirtyCheckoutErrorKind::ChallengeDrift,
        DirtyCheckoutErrorKind::ForeignLease,
        DirtyCheckoutErrorKind::MalformedState,
        DirtyCheckoutErrorKind::InvalidInput,
        DirtyCheckoutErrorKind::Timeout,
        DirtyCheckoutErrorKind::ResourceUnavailable,
        DirtyCheckoutErrorKind::SnapshotFailed,
        DirtyCheckoutErrorKind::AdoptionFailed,
        DirtyCheckoutErrorKind::RevocationFailed,
    ];
    assert_eq!(kinds.len(), 13);

    for kind in kinds {
        let (code, exit_code) = expected(kind);
        assert_eq!(kind.code(), code);
        assert_eq!(kind.exit_code(), exit_code);
        assert_eq!(error_kind_from_code(code), Some(kind));

        let cli_error = adoption_cli_error(domain_error(kind, "contract message"));
        assert_eq!(cli_error.code, code);
        assert_eq!(cli_error.exit_code, exit_code);
        assert_eq!(cli_error.message.as_ref(), "contract message");
        assert!(cli_error.hint.is_none());
        assert!(cli_error.details.is_none());

        let value = serde_json::to_value(super::super::super::error_envelope(
            "worktree.dirty-snapshot",
            &cli_error,
        ))
        .expect("serialize exact error envelope");
        let mut envelope_keys = value
            .as_object()
            .expect("error envelope object")
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        envelope_keys.sort_unstable();
        assert_eq!(envelope_keys, ["error", "ok", "schema_version"]);
        assert_eq!(
            value["schema_version"],
            "cli.git-cli.worktree.dirty-snapshot.v1"
        );
        assert_eq!(value["ok"], false);
        let mut error_keys = value["error"]
            .as_object()
            .expect("error object")
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        error_keys.sort_unstable();
        assert_eq!(error_keys, ["code", "message"]);
        assert_eq!(value["error"]["code"], code);
        assert_eq!(value["error"]["message"], "contract message");
    }
}
