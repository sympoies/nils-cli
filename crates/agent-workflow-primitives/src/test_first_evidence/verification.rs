use super::*;

pub(super) fn run_verify(args: VerifyArgs) -> i32 {
    let result = match verify_record(&args.common) {
        Ok(result) => result,
        Err(err) => {
            return render_error(
                VERIFY_SCHEMA_VERSION,
                VERIFY_COMMAND,
                args.common.format,
                err,
            );
        }
    };
    let mut missing = result.missing.clone();
    // Structural and subject readiness are independent; report both in one pass.
    let subject = args.project_path.as_deref().map(|project| {
        match_delivery_subject(
            &result.record,
            project,
            &args.remote,
            args.repository_id.as_deref(),
            "HEAD",
        )
    });
    if args.project_path.is_some() {
        match &result.record.subject {
            None => missing.extend([
                "baseline_subject".to_string(),
                "delivery_subject".to_string(),
            ]),
            Some(subject) if subject.deliveries.is_empty() => {
                missing.push("delivery_subject".to_string())
            }
            _ => {}
        }
        if let Some(Ok(subject)) = &subject
            && !subject.matches
            && subject.reason_code != "unbound-subject"
            && subject.reason_code != "delivery-subject-unbound"
        {
            missing.push("matching_delivery_subject".to_string());
        }
    }
    if !result.missing.is_empty() {
        let next_steps: Vec<_> = missing
            .iter()
            .map(|field| json!({"field": field, "command": recovery(field)}))
            .collect();
        let mut message = "test-first evidence record is incomplete".to_string();
        for field in &missing {
            message.push_str(&format!("\n  {field}: {}", recovery(field)));
        }
        return render_error(
            VERIFY_SCHEMA_VERSION,
            VERIFY_COMMAND,
            args.common.format,
            CliError::runtime(
                "incomplete-evidence",
                message,
                Some(json!({
                    "record_file": result.record_file, "missing": missing, "next_steps": next_steps,
                    "subject_error": subject.as_ref().and_then(|check| check.as_ref().err()).map(|err| &err.message),
                })),
            ),
        );
    }
    match subject {
        None => render_verify_success(args.common.format, &result),
        Some(Ok(subject)) if subject.matches => render_verify_success(args.common.format, &result),
        Some(Ok(subject)) => {
            let unbound = matches!(
                subject.reason_code.as_str(),
                "unbound-subject" | "delivery-subject-unbound"
            );
            let command = if result.record.subject.is_none() {
                "bind-baseline, then bind-delivery"
            } else {
                "bind-delivery"
            };
            render_error(
                VERIFY_SCHEMA_VERSION,
                VERIFY_COMMAND,
                args.common.format,
                CliError::data(
                    if unbound {
                        "unbound-subject"
                    } else {
                        "subject-mismatch"
                    },
                    format!(
                        "test-first evidence subject does not match the current delivery; run {command} after checking the baseline and repository"
                    ),
                    Some(json!({
                        "reason_code": subject.reason_code,
                        "missing": missing,
                        "next_steps": missing.iter().map(|field| json!({"field": field, "command": recovery(field)})).collect::<Vec<_>>(),
                    })),
                ),
            )
        }
        Some(Err(err)) => render_error(
            VERIFY_SCHEMA_VERSION,
            VERIFY_COMMAND,
            args.common.format,
            err,
        ),
    }
}

fn recovery(field: &str) -> &'static str {
    match field {
        "baseline_subject" => {
            "test-first-evidence bind-baseline (or retro-bind for retained pre-edit logs)"
        }
        "delivery_subject" | "matching_delivery_subject" => {
            "test-first-evidence bind-delivery after checking the repository and baseline"
        }
        "invalid_change_classification"
        | "contract_delta_blank_entry"
        | "changed_added_or_removed_behavior" => {
            "test-first-evidence init with an explicit contract delta before recording evidence"
        }
        "invalid_retro_binding" => {
            "inspect retro-binding provenance; re-import original retained logs into a fresh record"
        }
        field if field.contains("waiver") => {
            "test-first-evidence record-failing or record-waiver with complete rationale"
        }
        field if field.contains("failing") => {
            "test-first-evidence record-failing with meaningful before-fix evidence"
        }
        field if field.contains("final") => {
            "test-first-evidence record-final --status pass --scope focused (or the required scope); resolve-final for investigated failed attempts"
        }
        field if field.contains("residual_gap") => {
            "test-first-evidence record-gap --none or declare each gap with a follow-up"
        }
        _ => {
            "test-first-evidence record-impact with target, disposition, protected behavior, and rationale"
        }
    }
}
