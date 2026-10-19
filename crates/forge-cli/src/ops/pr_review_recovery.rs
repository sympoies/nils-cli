//! Recovery guidance for a retained native-review transaction.

use nils_common::cli_contract::schema_version_for;

use crate::backend::BackendRunner;
use crate::cli::{BINARY, PrReviewDecision};
use crate::error::ForgeError;
use crate::ops::pr_reviews;
use crate::provider::ProviderContext;

pub(super) struct PendingReviewRecovery<'a> {
    pub host: &'a str,
    pub repository: &'a str,
    pub number: u64,
    pub decision: PrReviewDecision,
    pub review_run_id: &'a str,
}

impl PendingReviewRecovery<'_> {
    pub fn creation_incomplete<R: BackendRunner>(
        &self,
        runner: &R,
        ctx: &ProviderContext,
        pr_url: &str,
        expected_head: &str,
        cause: ForgeError,
    ) -> ForgeError {
        // A failed response may follow a successful create. Discover only the
        // exact own receipt-bound transaction, and keep an unknown ID when a
        // provider read cannot prove it. This is diagnostic, never mutation.
        let review_id = (|| {
            let guards =
                pr_reviews::compute_pending_guards_for_pr(runner, ctx, self.number, pr_url).ok()?;
            if guards.head_sha != expected_head {
                return None;
            }
            let mut own = guards
                .reviews
                .iter()
                .filter(|review| review.viewer_did_author);
            let candidate = own.next()?;
            if own.next().is_some() {
                return None;
            }
            let snapshot =
                pr_reviews::compute_pending_snapshot(runner, ctx, &candidate.id).ok()??;
            (snapshot.number == self.number
                && snapshot.pr_url == pr_url
                && snapshot.review_id == candidate.id
                && snapshot.viewer_did_author
                && snapshot.head_sha == expected_head
                && snapshot.commit_sha.as_deref() == Some(expected_head)
                && snapshot.provenance == "receipt-bound"
                && snapshot.review_run_id.as_deref() == Some(self.review_run_id))
            .then_some(snapshot.review_id)
        })();
        self.incomplete(
            review_id.as_deref(),
            "pending review creation did not return a confirmed result",
            Some(cause),
        )
    }

    pub fn retained_error(&self, review_id: &str, cause: ForgeError) -> ForgeError {
        // Guard-specific validation codes remain public recovery contracts.
        // Enrich their details; backend failures use the transaction error.
        let guard = match &cause {
            ForgeError::Validation {
                schema_version,
                kind,
                message,
                ..
            } => Some((schema_version.clone(), *kind, message.clone())),
            _ => None,
        };
        let contextual = self.incomplete(
            Some(review_id),
            "pending review retained during recovery",
            Some(cause),
        );
        if let Some((schema, kind, message)) = guard {
            ForgeError::validation(
                schema,
                kind,
                message,
                contextual.detail().map(str::to_owned),
            )
        } else {
            contextual
        }
    }

    pub fn incomplete(
        &self,
        pending_review_id: Option<&str>,
        message: &str,
        cause: Option<ForgeError>,
    ) -> ForgeError {
        let mut detail = vec![
            format!("review_run_id={}", self.review_run_id),
            format!(
                "pending_review_id={}",
                pending_review_id.unwrap_or("<unknown>")
            ),
            "recovery=rerun the identical pr review command to append only missing findings"
                .to_string(),
        ];
        let prefix = format!(
            "forge-cli --provider github --host {} --repo {} --format json",
            shell_quote(self.host),
            shell_quote(self.repository),
        );
        if let Some(review_id) = pending_review_id {
            let target = format!("{} --review {}", self.number, shell_quote(review_id));
            let inspect = format!("{prefix} pr pending-review inspect {target}");
            // Obtain fresh CAS inputs from the complete inspect snapshot. The
            // user must explicitly choose discard; no recovery code runs it.
            let guards = concat!(
                "--expected-head \"$(printf '%s' \"$snapshot\" | jq -er '.data.snapshot.head_sha')\" ",
                "--expected-commit \"$(printf '%s' \"$snapshot\" | jq -er '.data.snapshot.commit_sha')\" ",
                "--expected-snapshot \"$(printf '%s' \"$snapshot\" | jq -er '.data.snapshot.snapshot_digest')\"",
            );
            detail.push(format!("inspect_command={inspect}"));
            detail.push("resume_submit_requires=complete receipt-bound inline manifest; otherwise rerun the identical pr review command".to_string());
            detail.push(format!(
                "resume_submit_command=snapshot=$({inspect}) && {prefix} pr pending-review resume-submit {target} --review-run-id {} --decision {} {guards}",
                shell_quote(self.review_run_id), self.decision.as_str(),
            ));
            detail.push(format!(
                "explicit_discard_command=snapshot=$({inspect}) && {prefix} pr pending-review discard {target} {guards} --confirm-discard --confirm-inline-content-loss",
            ));
        } else {
            detail.push(format!(
                "inspect_command={prefix} pr reviews {}",
                self.number
            ));
            detail.push("discard=inspect the exact own pending review ID before choosing pr pending-review discard; never delete an unconfirmed target".to_string());
        }
        if let Some(cause) = cause {
            detail.push(format!("cause_kind={}", cause.kind()));
            detail.push(format!("cause_message={}", cause.message()));
            if let Some(cause_detail) = cause.detail().filter(|value| !value.is_empty()) {
                detail.push(format!("cause_detail={cause_detail}"));
            }
        }
        ForgeError::validation(
            schema_version_for(BINARY, "error", 1),
            "pending_review_transaction_incomplete",
            message,
            Some(detail.join("; ")),
        )
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
