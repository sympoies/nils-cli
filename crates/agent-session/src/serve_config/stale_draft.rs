//! The single stale-draft setting, resolved before serve starts threads.
use super::*;
use crate::send_guard::drafts::{BOUND_ENV, parse_bound};

pub(super) fn resolve(
    value: Option<&Value>,
    lookup: &dyn Fn(&str) -> Option<OsString>,
    warnings: &mut Vec<String>,
    assignments: &mut Vec<(&'static str, OsString)>,
) -> Result<Source, CliError> {
    let file = value
        .map(|value| {
            value.as_u64().ok_or_else(|| {
                invalid_value("stale_draft_seconds", "must be unsigned integer seconds")
            })
        })
        .transpose()?;
    let environment = lookup(BOUND_ENV)
        .map(|value| {
            value.into_string().map_err(|_| {
                keyed_error(
                    "serve-config-invalid-value",
                    "stale_draft_seconds",
                    "environment",
                    "must be unsigned integer seconds",
                )
            })
        })
        .transpose()?;
    if let Some(raw) = environment {
        parse_bound(&raw).map_err(|_| {
            keyed_error(
                "serve-config-invalid-value",
                "stale_draft_seconds",
                "environment",
                "must be unsigned integer seconds",
            )
        })?;
        if file.is_some() {
            warnings.push(format!(
                "{BOUND_ENV} overrides the config file's stale_draft_seconds"
            ));
        }
        return Ok(Source::Environment);
    }
    if let Some(seconds) = file {
        assignments.push((BOUND_ENV, OsString::from(seconds.to_string())));
        return Ok(Source::File);
    }
    Ok(Source::None)
}
