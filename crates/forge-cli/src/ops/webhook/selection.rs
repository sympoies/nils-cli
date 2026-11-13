//! Mutually exclusive exact-id and window selection, validated before POST.
use std::collections::HashSet;

use jiff::Timestamp;

use super::engine::{invalid_response, schema_err};
use super::redeliver::outcome;
use crate::cli::HookDeliveryStatus;
use crate::error::ForgeError;

#[derive(Debug, Clone)]
pub(super) enum Selection {
    Exact(Vec<u64>),
    Window {
        status: HookDeliveryStatus,
        since: Timestamp,
        until: Timestamp,
    },
}

impl Selection {
    pub(super) fn new(
        ids: Option<Vec<u64>>,
        status: Option<HookDeliveryStatus>,
        since: Option<&str>,
        until: Option<&str>,
    ) -> Result<Self, ForgeError> {
        if let Some(ids) = ids {
            if ids.is_empty()
                || ids.contains(&0)
                || status.is_some()
                || since.is_some()
                || until.is_some()
            {
                return Err(ForgeError::validation(
                    schema_err(),
                    "hook_selection_invalid",
                    "delivery ids must be nonempty, positive and used without status or window filters",
                    None,
                ));
            }
            return Ok(Self::Exact(ids));
        }
        let invalid = || {
            ForgeError::validation(
                schema_err(),
                "hook_window_invalid",
                "since and until must be RFC 3339 timestamps with since before until",
                None,
            )
        };
        let since = since
            .ok_or_else(invalid)?
            .parse::<Timestamp>()
            .map_err(|_| invalid())?;
        let until = until
            .ok_or_else(invalid)?
            .parse::<Timestamp>()
            .map_err(|_| invalid())?;
        if since >= until {
            return Err(invalid());
        }
        Ok(Self::Window {
            status: status.unwrap_or_default(),
            since,
            until,
        })
    }

    pub(super) fn validate_ids(&self, known: &HashSet<u64>) -> Result<(), ForgeError> {
        if let Self::Exact(ids) = self {
            for id in ids {
                if !known.contains(id) {
                    return Err(ForgeError::runtime_failure(
                        schema_err(),
                        "hook_delivery_not_found",
                        format!(
                            "delivery {id} was not found on this hook; no deliveries attempted"
                        ),
                        None,
                    ));
                }
            }
        }
        Ok(())
    }

    pub(super) fn contains(
        &self,
        id: u64,
        code: Option<u16>,
        at: &str,
    ) -> Result<bool, ForgeError> {
        match self {
            Self::Exact(ids) => Ok(ids.contains(&id)),
            Self::Window {
                status,
                since,
                until,
            } => {
                let at = at
                    .parse::<Timestamp>()
                    .map_err(|_| invalid_response("delivery timestamp"))?;
                let selected = match status {
                    HookDeliveryStatus::All => true,
                    HookDeliveryStatus::Success => outcome(code) == "success",
                    HookDeliveryStatus::Failed => outcome(code) == "failed",
                };
                Ok(selected && at >= *since && at < *until)
            }
        }
    }
}
