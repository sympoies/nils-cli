use super::*;

pub(super) fn run_record_final(args: RecordFinalArgs) -> i32 {
    let format = args.common.format;
    let full_record = args.output.full_record;
    match update_record_with_item(args.common.out_dir.as_path(), |record| {
        let command = required_text(Some(&args.command), "--command", "missing-final-command")?;
        let command = redact_text(command).value;
        let next_attempt = record
            .final_validations
            .iter()
            .filter(|item| item.command.trim() == command && item.scope == Some(args.scope))
            .map(|item| item.attempt)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| {
                CliError::data(
                    "final-validation-attempt-overflow",
                    "final validation attempt counter is exhausted",
                    None,
                )
            })?;
        let validation = FinalValidation {
            command,
            status: args.status,
            scope: Some(args.scope),
            attempt: next_attempt,
            resolution: None,
            summary: args
                .summary
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| redact_text(value).value),
            artifacts: normalized_paths(&args.artifact),
        };
        let latest = record
            .final_validations
            .iter()
            .filter(|item| {
                item.command.trim() == validation.command && item.scope == validation.scope
            })
            .max_by_key(|item| item.attempt);
        if latest.is_some_and(|item| item.status == validation.status) {
            return Err(CliError::data(
                "duplicate-final-validation",
                "latest final validation attempt already has this command, scope, and status",
                None,
            ));
        }
        record.final_validations.push(validation.clone());
        record.final_validations.sort_by(|left, right| {
            (left.scope, left.command.trim(), left.attempt).cmp(&(
                right.scope,
                right.command.trim(),
                right.attempt,
            ))
        });
        Ok(validation)
    }) {
        Ok((result, item)) => render_mutation_success(
            RECORD_FINAL_RECEIPT_SCHEMA_VERSION,
            RECORD_FINAL_SCHEMA_VERSION,
            RECORD_FINAL_COMMAND,
            format,
            full_record,
            &result,
            &item,
        ),
        Err(err) => render_error(
            RECORD_FINAL_SCHEMA_VERSION,
            RECORD_FINAL_COMMAND,
            format,
            err,
        ),
    }
}

#[derive(Debug, clap::Args)]
pub struct ResolveFinalArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Command of the failed final validation.
    #[arg(long)]
    pub command: String,
    /// Scope of the failed final validation.
    #[arg(long, value_enum)]
    pub scope: ValidationScope,
    /// Failed attempt number, as shown by `show --format json`.
    #[arg(long)]
    pub attempt: u32,
    /// Passing canonical validation that covers the failed target on the same source.
    #[arg(long)]
    pub passing_command: String,
    /// Scope of the passing canonical validation.
    #[arg(long, value_enum)]
    pub passing_scope: ValidationScope,
    /// Passing attempt number, as shown by `show --format json`.
    #[arg(long)]
    pub passing_attempt: u32,
    /// Why this passing run resolves the failure, including scope and source equivalence.
    #[arg(long)]
    pub reason: String,
    /// Investigation evidence explaining the original failure. Repeatable.
    #[arg(long, required = true)]
    pub investigation: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FinalResolution {
    pub passing_command: String,
    pub passing_scope: ValidationScope,
    pub passing_attempt: u32,
    pub reason: String,
    pub investigation: Vec<String>,
}

pub(super) fn valid_resolution_history(record: &EvidenceRecord, failed: &FinalValidation) -> bool {
    let Some(resolution) = &failed.resolution else {
        return false;
    };
    failed.status == ValidationStatus::Fail
        && !resolution.reason.trim().is_empty()
        && !resolution.investigation.is_empty()
        && resolution
            .investigation
            .iter()
            .all(|s| !s.trim().is_empty())
        && resolution.passing_attempt > 0
        && record.final_validations.iter().any(|passing| {
            passing.command.trim() == resolution.passing_command.trim()
                && passing.scope == Some(resolution.passing_scope)
                && passing.attempt == resolution.passing_attempt
                && passing.status == ValidationStatus::Pass
        })
}

fn valid_resolution(record: &EvidenceRecord, failed: &FinalValidation) -> bool {
    let Some(resolution) = &failed.resolution else {
        return false;
    };
    valid_resolution_history(record, failed)
        && effective_final_validations(record)
            .get(&(
                Some(resolution.passing_scope),
                resolution.passing_command.trim(),
            ))
            .is_some_and(|passing| passing.status == ValidationStatus::Pass)
}

pub(super) fn unresolved_failure(record: &EvidenceRecord, item: &FinalValidation) -> bool {
    item.status == ValidationStatus::Fail && !valid_resolution(record, item)
}

pub(super) fn run_resolve_final(args: ResolveFinalArgs) -> i32 {
    let format = args.common.format;
    let result = update_record(args.common.out_dir.as_path(), |record| {
        let command = redact_text(required_text(
            Some(&args.command),
            "--command",
            "missing-final-command",
        )?)
        .value;
        let passing_command = redact_text(required_text(
            Some(&args.passing_command),
            "--passing-command",
            "missing-passing-command",
        )?)
        .value;
        let reason = redact_text(required_text(
            Some(&args.reason),
            "--reason",
            "missing-resolution-reason",
        )?)
        .value;
        if args.investigation.is_empty() || args.investigation.iter().any(|s| s.trim().is_empty()) {
            return Err(CliError::data(
                "missing-investigation",
                "at least one non-empty --investigation is required",
                None,
            ));
        }
        let index = record
            .final_validations
            .iter()
            .position(|item| {
                item.command == command
                    && item.scope == Some(args.scope)
                    && item.attempt == args.attempt
            })
            .ok_or_else(|| {
                CliError::data(
                    "failed-final-not-found",
                    "failed command, scope, and attempt must identify an existing final validation",
                    None,
                )
            })?;
        if record.final_validations[index].status != ValidationStatus::Fail {
            return Err(CliError::data(
                "final-not-failed",
                "only a failed final validation can be resolved",
                None,
            ));
        }
        if record.final_validations[index].resolution.is_some() {
            return Err(CliError::data(
                "final-already-resolved",
                "the failed final attempt already has a disposition; preserve it and record a new validation attempt",
                None,
            ));
        }
        let latest = effective_final_validations(record);
        let passing_is_current = latest
            .get(&(Some(args.passing_scope), passing_command.trim()))
            .is_some_and(|passing| passing.attempt == args.passing_attempt);
        record.final_validations[index].resolution = Some(FinalResolution {
            passing_command,
            passing_scope: args.passing_scope,
            passing_attempt: args.passing_attempt,
            reason,
            investigation: redacted_strings(&args.investigation),
        });
        if !passing_is_current || !valid_resolution(record, &record.final_validations[index]) {
            return Err(CliError::data(
                "invalid-final-resolution",
                "the referenced command, scope, and attempt must be the latest passing validation for that identity",
                None,
            ));
        }
        Ok(())
    });
    match result {
        Ok(result) => render_record_success(
            "cli.test-first-evidence.resolve-final.v1",
            "test-first-evidence resolve-final",
            format,
            &result,
        ),
        Err(err) => render_error(
            "cli.test-first-evidence.resolve-final.v1",
            "test-first-evidence resolve-final",
            format,
            err,
        ),
    }
}

// Consume matches from the original text so a rendered tail is never rewritten again.
fn portable_investigation(reference: &str) -> String {
    static URI: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let uri = URI.get_or_init(|| {
        regex::Regex::new(r#"[A-Za-z][A-Za-z0-9+.-]*:[^\s<>"`]+"#)
            .expect("valid investigation URI regex")
    });
    let uri_ranges: Vec<_> = uri.find_iter(reference).map(|uri| uri.range()).collect();
    let hits = nils_common::provider_payload::scan_local_paths(reference);
    let mut remaining = reference;
    let mut display = String::new();
    while let Some((start, hit)) = hits
        .iter()
        .filter_map(|hit| remaining.find(&hit.sample).map(|start| (start, hit)))
        .min_by_key(|(start, hit)| (*start, std::cmp::Reverse(hit.sample.len())))
    {
        display.push_str(&remaining[..start]);
        let original_start = reference.len() - remaining.len() + start;
        // URI components are audit-link text, not standalone filesystem references.
        // Leave them intact so the unchanged publication guard can refuse them.
        if uri_ranges
            .iter()
            .any(|range| range.contains(&original_start))
        {
            display.push_str(&hit.sample);
        } else {
            display.push_str(&hit.suggestion);
        }
        remaining = &remaining[start + hit.sample.len()..];
    }
    display.push_str(remaining);
    display
}

/// Portable audit section for PR bodies containing explicit failed-final dispositions.
pub fn final_resolution_summary(record: &EvidenceRecord) -> String {
    if !record
        .final_validations
        .iter()
        .any(|v| v.resolution.is_some())
    {
        return String::new();
    }
    let mut summary = String::from("## Final validation dispositions\n");
    for validation in &record.final_validations {
        summary.push_str(&format!(
            "\n- {}: `{}` (scope {}, attempt {}).",
            validation.status.as_str().to_uppercase(),
            validation.command,
            validation.scope.map_or("unknown", ValidationScope::as_str),
            validation.attempt
        ));
        if let Some(detail) = &validation.summary {
            summary.push_str(&format!(" {detail}"));
        }
        if let Some(resolution) = &validation.resolution {
            summary.push_str(&format!(
                " Resolved by `{}` (scope {}, attempt {}): {}. Investigation: {}.",
                resolution.passing_command,
                resolution.passing_scope.as_str(),
                resolution.passing_attempt,
                resolution.reason,
                resolution
                    .investigation
                    .iter()
                    .map(|reference| portable_investigation(reference))
                    .collect::<Vec<_>>()
                    .join("; ")
            ));
        }
    }
    summary.push('\n');
    let summary = redact_text(&summary).value;
    // Persisted records can come from producers that did not redact assignments.
    // Apply compound-key masking only at export so stored command identities and
    // their resolution references remain unchanged.
    static COMPOUND_SECRET: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let assignments = COMPOUND_SECRET.get_or_init(|| {
        regex::Regex::new(
            r#"(?ix)
            (?P<key>\b(?:[a-z0-9_-]*[_-])?(?:access[_-]?token|refresh[_-]?token|api[_-]?key|apikey|authorization|cookie|password|secret|session[_-]?id|token)(?:[_-][a-z0-9_-]+)*\b)
            (?P<after_key>"?\s*[:=]\s*)
            (?P<value>"[^"]*"|'[^']*'|[^\s,;&}`]+)
            "#,
        )
        .expect("valid compound credential assignment regex")
    });
    assignments
        .replace_all(&summary, |captures: &regex::Captures<'_>| {
            if &captures["value"] == "[REDACTED]" {
                captures[0].to_string()
            } else {
                format!("{}{}[REDACTED]", &captures["key"], &captures["after_key"])
            }
        })
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::{assert_eq, assert_ne};

    fn fixture(dir: &Path) {
        fs::write(dir.join(RECORD_FILE_NAME), serde_json::to_vec(&json!({
            "schema_version": RECORD_SCHEMA_VERSION,
            "change_classification": "bug-fix",
            "contract_delta": {"changed_behaviors": ["resolve failed final validation"]},
            "no_existing_tests_reason": "no prior owner",
            "failing_tests": [{"command": "cargo test regression", "exit_code": 101, "summary": "regression", "expected_failure": "missing resolution", "observed_failure": "refused"}],
            "final_validations": [
                {"command": "cargo test affected", "scope": "affected-suite", "attempt": 1, "status": "fail"},
                {"command": "canonical", "scope": "focused", "attempt": 1, "status": "pass"}
            ],
            "no_residual_gaps": true
        })).unwrap()).unwrap();
    }

    fn resolve(dir: &Path, passing: &str, attempt: &str, reason: &str, investigation: &str) -> i32 {
        run_with_args([
            "test-first-evidence",
            "resolve-final",
            "--out",
            dir.to_str().unwrap(),
            "--command",
            "cargo test affected",
            "--scope",
            "affected-suite",
            "--attempt",
            "1",
            "--passing-command",
            passing,
            "--passing-scope",
            "focused",
            "--passing-attempt",
            attempt,
            "--reason",
            reason,
            "--investigation",
            investigation,
            "--format",
            "json",
        ])
    }

    #[test]
    fn invalid_dispositions_do_not_mutate_history() {
        for (passing, attempt, reason, investigation) in [
            ("absent", "1", "covers the target", "isolated reproduction"),
            (
                "canonical",
                "2",
                "covers the target",
                "isolated reproduction",
            ),
            (
                "canonical",
                "0",
                "covers the target",
                "isolated reproduction",
            ),
            ("canonical", "1", " ", "isolated reproduction"),
            ("canonical", "1", "covers the target", " "),
        ] {
            let dir = tempfile::tempdir().unwrap();
            fixture(dir.path());
            let before = fs::read(dir.path().join(RECORD_FILE_NAME)).unwrap();
            assert_ne!(
                resolve(dir.path(), passing, attempt, reason, investigation),
                0
            );
            assert_eq!(fs::read(dir.path().join(RECORD_FILE_NAME)).unwrap(), before);
        }
    }

    #[test]
    fn final_disposition_summary_redacts_persisted_secret_shapes() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        assert_eq!(
            resolve(
                dir.path(),
                "canonical",
                "1",
                "same source and covered target",
                "isolated investigation"
            ),
            0
        );
        let mut result = verify_dir(dir.path()).unwrap();
        let synthetic = format!("ghp_{}", "fixture".repeat(3));
        let failed = result
            .record
            .final_validations
            .iter_mut()
            .find(|v| v.resolution.is_some())
            .unwrap();
        failed
            .resolution
            .as_mut()
            .unwrap()
            .investigation
            .push(synthetic.clone());
        let summary = final_resolution_summary(&result.record);
        assert!(!summary.contains(&synthetic));
        assert!(summary.contains("[REDACTED]"));
    }

    #[test]
    fn persisted_resolution_on_pass_is_not_admissible() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        let file = dir.path().join(RECORD_FILE_NAME);
        let mut record: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        record["final_validations"][0]["status"] = json!("pass");
        record["final_validations"][1]["resolution"] = json!({
            "passing_command": "canonical", "passing_scope": "focused", "passing_attempt": 1,
            "reason": "self reference on pass", "investigation": ["invalid owner"]
        });
        fs::write(&file, serde_json::to_vec(&record).unwrap()).unwrap();
        let result = verify_dir(dir.path()).unwrap();
        assert!(
            result
                .missing
                .iter()
                .any(|s| s == "invalid_final_resolution")
        );
    }

    #[test]
    fn failed_and_superseded_passing_references_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        for status in ["fail", "pass"] {
            assert_eq!(
                run_with_args([
                    "test-first-evidence",
                    "record-final",
                    "--out",
                    dir.path().to_str().unwrap(),
                    "--command",
                    "canonical",
                    "--scope",
                    "focused",
                    "--status",
                    status
                ]),
                0
            );
            let before = fs::read(dir.path().join(RECORD_FILE_NAME)).unwrap();
            assert_eq!(
                resolve(
                    dir.path(),
                    "canonical",
                    "1",
                    "covers target",
                    "investigated failure"
                ),
                EXIT_DATA
            );
            assert_eq!(fs::read(dir.path().join(RECORD_FILE_NAME)).unwrap(), before);
        }
        assert_eq!(
            resolve(
                dir.path(),
                "canonical",
                "3",
                "covers target",
                "investigated failure"
            ),
            0
        );
    }

    #[test]
    fn disposition_is_immutable_and_later_failure_reopens_readiness() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        assert_eq!(
            resolve(
                dir.path(),
                "canonical",
                "1",
                "same source and covered target",
                "isolated investigation"
            ),
            0
        );
        let before = fs::read(dir.path().join(RECORD_FILE_NAME)).unwrap();
        assert!(verify_dir(dir.path()).unwrap().complete);
        assert_eq!(
            resolve(
                dir.path(),
                "canonical",
                "1",
                "different reason",
                "different investigation"
            ),
            EXIT_DATA
        );
        assert_eq!(fs::read(dir.path().join(RECORD_FILE_NAME)).unwrap(), before);
        assert_eq!(
            run_with_args([
                "test-first-evidence",
                "record-final",
                "--out",
                dir.path().to_str().unwrap(),
                "--command",
                "canonical",
                "--scope",
                "focused",
                "--status",
                "fail"
            ]),
            0
        );
        let result = verify_dir(dir.path()).unwrap();
        assert!(
            result
                .missing
                .iter()
                .any(|s| s == "failed_final_validation")
        );
        assert_eq!(
            run_with_args([
                "test-first-evidence",
                "record-final",
                "--out",
                dir.path().to_str().unwrap(),
                "--command",
                "canonical",
                "--scope",
                "focused",
                "--status",
                "pass"
            ]),
            0
        );
        assert!(verify_dir(dir.path()).unwrap().complete);
        assert_eq!(result.record.final_validations.len(), 3);
        assert!(
            result
                .record
                .final_validations
                .iter()
                .find(|v| v.command == "cargo test affected")
                .unwrap()
                .resolution
                .is_some()
        );
    }
}
