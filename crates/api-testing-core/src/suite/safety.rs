use std::path::Path;

use crate::Result;

pub const MSG_NOT_SELECTED: &str = "not_selected";
pub const MSG_SKIPPED_BY_ID: &str = "skipped_by_id";
pub const MSG_TAG_MISMATCH: &str = "tag_mismatch";

pub const MSG_WRITE_CASES_DISABLED: &str = "write_cases_disabled";
pub const MSG_WRITE_CAPABLE_REQUIRES_ALLOW_WRITE_TRUE: &str =
    "write_capable_case_requires_allowWrite_true";
pub const MSG_MUTATION_REQUIRES_ALLOW_WRITE_TRUE: &str = "mutation_case_requires_allowWrite_true";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafetyDecision {
    Allow,
    Skip(&'static str),
    Fail(&'static str),
}

pub fn writes_enabled(allow_writes_flag: bool, effective_env: &str) -> bool {
    allow_writes_flag || effective_env.trim().eq_ignore_ascii_case("local")
}

pub fn rest_method_is_write(method: &str) -> bool {
    let m = method.trim().to_ascii_uppercase();
    !matches!(m.as_str(), "GET" | "HEAD" | "OPTIONS")
}

pub fn graphql_operation_is_write_capable(
    operation_file: &Path,
    allow_write: bool,
) -> Result<bool> {
    if allow_write {
        return Ok(true);
    }
    crate::graphql::mutation::operation_file_is_mutation(operation_file)
}

pub fn graphql_safety_decision(
    operation_file: &Path,
    allow_write: bool,
    allow_writes_flag: bool,
    effective_env: &str,
) -> Result<SafetyDecision> {
    let writes_enabled = writes_enabled(allow_writes_flag, effective_env);

    // Defensive: explicit allowWrite=true is treated as write-capable even if mutation detection fails.
    if allow_write && !writes_enabled {
        return Ok(SafetyDecision::Skip(MSG_WRITE_CASES_DISABLED));
    }

    let is_mutation = crate::graphql::mutation::operation_file_is_mutation(operation_file)?;
    if !is_mutation {
        return Ok(SafetyDecision::Allow);
    }

    if !allow_write {
        return Ok(SafetyDecision::Fail(MSG_MUTATION_REQUIRES_ALLOW_WRITE_TRUE));
    }

    if !writes_enabled {
        return Ok(SafetyDecision::Skip(MSG_WRITE_CASES_DISABLED));
    }

    Ok(SafetyDecision::Allow)
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;

    #[test]
    fn suite_safety_rest_method_write_detection_matches_script_intent() {
        assert!(!rest_method_is_write("GET"));
        assert!(!rest_method_is_write("head"));
        assert!(!rest_method_is_write(" OPTIONS "));
        assert!(rest_method_is_write("POST"));
        assert!(rest_method_is_write("PATCH"));
        assert!(rest_method_is_write("DELETE"));
    }

    #[test]
    fn suite_safety_graphql_allow_write_true_is_write_capable() {
        let tmp = TempDir::new().unwrap();
        let op = tmp.path().join("q.graphql");
        std::fs::write(&op, "query Q { ok }\n").unwrap();

        assert!(graphql_operation_is_write_capable(&op, true).unwrap());
    }
}
