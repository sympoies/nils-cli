use crate::error::CliError;
use crate::model::{AxSelector, AxTarget};

#[derive(Debug, Clone, Default)]
pub struct AxSelectorInput {
    pub node_id: Option<String>,
    pub role: Option<String>,
    pub title_contains: Option<String>,
    pub identifier_contains: Option<String>,
    pub value_contains: Option<String>,
    pub subrole: Option<String>,
    pub focused: Option<bool>,
    pub enabled: Option<bool>,
    pub nth: Option<u32>,
}

pub fn build_target(
    session_id: Option<String>,
    app: Option<String>,
    bundle_id: Option<String>,
    window_title_contains: Option<String>,
) -> Result<AxTarget, CliError> {
    let mut target_count = 0;
    if session_id.is_some() {
        target_count += 1;
    }
    if app.is_some() {
        target_count += 1;
    }
    if bundle_id.is_some() {
        target_count += 1;
    }

    if target_count > 1 {
        return Err(CliError::usage(
            "--session-id cannot be combined with --app/--bundle-id",
        ));
    }

    Ok(AxTarget {
        session_id,
        app,
        bundle_id,
        window_title_contains,
    })
}

pub fn build_selector(input: AxSelectorInput) -> Result<AxSelector, CliError> {
    if input.nth == Some(0) {
        return Err(CliError::usage("--nth must be at least 1"));
    }

    let has_primary_filters = input.role.is_some()
        || input.title_contains.is_some()
        || input.identifier_contains.is_some()
        || input.value_contains.is_some()
        || input.subrole.is_some()
        || input.focused.is_some()
        || input.enabled.is_some();
    let has_non_node_filters = has_primary_filters || input.nth.is_some();

    if input.node_id.is_some() && has_non_node_filters {
        return Err(CliError::usage(
            "--node-id cannot be combined with role/title/identifier/value/subrole/focused/enabled/nth selectors",
        ));
    }

    if input.node_id.is_none() && !has_primary_filters {
        if input.nth.is_some() {
            return Err(CliError::usage(
                "--nth requires at least one selector filter when --node-id is not set",
            ));
        }
        return Err(CliError::usage(
            "provide --node-id or at least one selector filter (--role/--title-contains/--identifier-contains/--value-contains/--subrole/--focused/--enabled)",
        ));
    }

    Ok(AxSelector {
        node_id: input.node_id,
        role: input.role,
        title_contains: input.title_contains,
        identifier_contains: input.identifier_contains,
        value_contains: input.value_contains,
        subrole: input.subrole,
        focused: input.focused,
        enabled: input.enabled,
        nth: input.nth.map(|value| value as usize),
    })
}
