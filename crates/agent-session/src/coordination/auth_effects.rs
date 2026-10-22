//! Recovery records selectors, never tool arguments or execution capabilities.
use super::*;

#[cfg(all(test, target_os = "linux"))]
pub(crate) mod test_support;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UnknownOperation {
    pub lease_id: String,
    pub claim_id: String,
    pub revision: u64,
    pub state: String,
}

pub(crate) fn snapshot(
    context: &CliContext,
    record: &SessionRecord,
) -> Result<(usize, Vec<UnknownOperation>), CliError> {
    let incarnation = incarnation(record)?;
    let locked = lock_registry_observational(context)?;
    let operations: Vec<_> = locked
        .registry
        .operations
        .iter()
        .filter(|op| {
            op.session_id == record.id
                && op.session_incarnation == incarnation
                && (matches!(op.state.as_str(), "active" | "completing" | "reconcile_pending")
                    // The provider's exit wrapper can retire leases before the
                    // controller observes stop. Retirement proves no external
                    // effect outcome, so retain those unknowns as well.
                    || op.state == "abandoned" && op.outcome.is_none())
        })
        .collect();
    Ok((
        operations.len(),
        operations
            .into_iter()
            .take(8)
            .map(|op| UnknownOperation {
                lease_id: op.lease_id.clone(),
                claim_id: op.claim_id.clone(),
                revision: op.revision,
                state: "unknown".into(),
            })
            .collect(),
    ))
}

pub(super) fn quarantined(registry: &Path, id: &str, incarnation: &str) -> bool {
    let Some(state_dir) = registry.parent().and_then(Path::parent) else {
        return true;
    };
    let context = CliContext {
        state_dir: state_dir.into(),
        host: None,
    };
    match crate::load_session_record(&context, id) {
        Ok(record)
            if record
                .runtime
                .as_ref()
                .is_some_and(|r| r.launch_id == incarnation) =>
        {
            crate::auth_incident::recovery::ensure_input_allowed(&context, &record).is_err()
        }
        _ => true,
    }
}
