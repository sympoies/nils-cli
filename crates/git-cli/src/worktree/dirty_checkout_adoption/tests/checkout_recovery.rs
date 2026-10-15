use super::*;

#[test]
fn checkout_wide_recovery_prunes_once_after_processing_all_pending_records() {
    let records: Vec<_> = (0..MAX_RECOVERY_STATE_ENTRIES)
        .map(|index| (format!("token-{index}"), format!("receipt-{index}")))
        .collect();
    let recovered = std::cell::Cell::new(0_usize);
    let pruned = std::cell::Cell::new(0_usize);

    recover_pending_batch_with(
        records,
        "token-0",
        |_| {
            recovered.set(recovered.get() + 1);
            Ok(PendingRecovery::Revoked)
        },
        |_, _| panic!("revoked records do not require staged rollback"),
        |failed_receipt_id| {
            assert!(failed_receipt_id.is_none());
            pruned.set(pruned.get() + 1);
            Ok(())
        },
    )
    .expect("checkout-wide recovery succeeds");

    assert_eq!(recovered.get(), MAX_RECOVERY_STATE_ENTRIES - 1);
    assert_eq!(
        pruned.get(),
        1,
        "retention pruning must not rescan the checkout after every recovered marker"
    );
}

#[test]
fn checkout_wide_recovery_prunes_after_failed_staged_rollback() {
    let state_entries = std::cell::Cell::new(MAX_RECOVERY_STATE_ENTRIES);
    let pruned = std::cell::Cell::new(0_usize);

    let error = recover_pending_batch_with(
        [("staged-token".to_string(), "staged-receipt".to_string())],
        "current-token",
        |_| Ok(PendingRecovery::Staged),
        |_, _| {
            state_entries.set(state_entries.get() + 1);
            Err(domain_error(
                DirtyCheckoutErrorKind::AdoptionFailed,
                "staged rollback failed after durable tombstone rename",
            ))
        },
        |failed_receipt_id| {
            assert_eq!(failed_receipt_id, Some("staged-receipt"));
            pruned.set(pruned.get() + 1);
            state_entries.set(state_entries.get() - 1);
            Ok(())
        },
    )
    .expect_err("the staged rollback failure remains primary");

    assert!(
        error
            .to_string()
            .contains("staged rollback failed after durable tombstone rename")
    );
    assert_eq!(pruned.get(), 1, "deferred pruning must still run");
    assert_eq!(state_entries.get(), MAX_RECOVERY_STATE_ENTRIES);
}

#[test]
fn checkout_wide_recovery_preserves_primary_error_when_pruning_also_fails() {
    let error = recover_pending_batch_with(
        [("staged-token".to_string(), "staged-receipt".to_string())],
        "current-token",
        |_| Ok(PendingRecovery::Staged),
        |_, _| {
            Err(domain_error(
                DirtyCheckoutErrorKind::AdoptionFailed,
                "unique primary recovery failure",
            ))
        },
        |failed_receipt_id| {
            assert_eq!(failed_receipt_id, Some("staged-receipt"));
            Err(domain_error(
                DirtyCheckoutErrorKind::ResourceUnavailable,
                "unique deferred prune failure",
            ))
        },
    )
    .expect_err("both recovery and pruning fail");

    assert_eq!(
        error
            .downcast_ref::<DirtyCheckoutError>()
            .expect("primary typed error remains in the error chain")
            .kind(),
        DirtyCheckoutErrorKind::AdoptionFailed
    );
    let diagnostic = error.to_string();
    assert!(diagnostic.contains("unique primary recovery failure"));
    assert!(diagnostic.contains("unique deferred prune failure"));
}
