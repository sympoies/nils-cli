use super::*;
use cli::RecordFailingArgs;

const RECORD_FAILING_SCHEMA_VERSION: &str = "cli.test-first-evidence.record-failing.v2";
const RECORD_FAILING_RECEIPT_SCHEMA_VERSION: &str = "cli.test-first-evidence.record-failing.v3";
const RECORD_FAILING_COMMAND: &str = "test-first-evidence record-failing";

pub(super) fn run_record_failing(args: RecordFailingArgs) -> i32 {
    let format = args.common.format;
    let full_record = args.output.full_record;
    match update_record_with_item(args.common.out_dir.as_path(), |record| {
        let command = required_text(Some(&args.command), "--command", "missing-failing-command")?;
        let summary = required_text(Some(&args.summary), "--summary", "missing-failing-summary")?;
        let expected_failure = required_text(
            Some(&args.expected_failure),
            "--expected-failure",
            "missing-expected-failure",
        )?;
        let observed_failure = required_text(
            Some(&args.observed_failure),
            "--observed-failure",
            "missing-observed-failure",
        )?;
        if let Some((code, message)) = invalid_failure(args.exit_code, observed_failure) {
            return Err(CliError::data(code, message, None));
        }
        if !observed_failure.lines().any(is_assertion_result) {
            return Err(CliError::data(
                "failing-test-unclassified",
                "the output has no recognized assertion result. Supply the runner output or use record-waiver with substitute validation",
                None,
            ));
        }
        let evidence = FailingEvidence {
            command: redact_text(command).value,
            exit_code: args.exit_code,
            summary: redact_text(summary).value,
            expected_failure: redact_text(expected_failure).value,
            observed_failure: redact_text(observed_failure).value,
            test_name: args
                .test_name
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| redact_text(value).value),
            artifacts: normalized_paths(&args.artifact),
        };
        let duplicate = record.failing_tests.iter().any(|item| {
            item.command.trim() == evidence.command
                && item.test_name.as_deref().map(str::trim) == evidence.test_name.as_deref()
        });
        if duplicate {
            return Err(CliError::data(
                "duplicate-failing-evidence",
                "failing evidence with the same command and test name already exists",
                None,
            ));
        }
        record.failing_tests.push(evidence.clone());
        record.failing_tests.sort_by(|left, right| {
            (&left.test_name, &left.command).cmp(&(&right.test_name, &right.command))
        });
        Ok(evidence)
    }) {
        Ok((result, item)) => render_mutation_success(
            RECORD_FAILING_RECEIPT_SCHEMA_VERSION,
            RECORD_FAILING_SCHEMA_VERSION,
            RECORD_FAILING_COMMAND,
            format,
            full_record,
            &result,
            &item,
        ),
        Err(err) => render_error(
            RECORD_FAILING_SCHEMA_VERSION,
            RECORD_FAILING_COMMAND,
            format,
            err,
        ),
    }
}

// Use diagnostic lines and runner result markers. A test's name is not a result.
pub(super) fn invalid_failure(
    exit_code: i32,
    observed: &str,
) -> Option<(&'static str, &'static str)> {
    let observed = observed.to_ascii_lowercase();
    let lines = observed.lines().map(str::trim);
    let has_diagnostic = |prefixes: &[&str]| {
        lines
            .clone()
            .filter(|line| !is_assertion_result(line))
            .map(compiler_diagnostic)
            .any(|diagnostic| prefixes.iter().any(|prefix| diagnostic.starts_with(prefix)))
    };
    if exit_code == 124 || lines.clone().any(is_timeout_result) {
        Some((
            "failing-test-timeout",
            "the run timed out. Record the expected assertion failure after the test runs",
        ))
    } else if has_diagnostic(&[
        "missing symbol",
        "undefined symbol",
        "undefined reference",
        "cannot find symbol",
        "error[e0412]",
        "error[e0425]",
        "error[e0432]",
        "error[e0433]",
    ]) {
        Some((
            "failing-test-missing-symbol",
            "the run failed on a missing symbol. Make the test runnable before recording the expected assertion failure",
        ))
    } else if has_diagnostic(&[
        "compile error",
        "compiler error",
        "compilation error",
        "compilation failed",
        "error[e",
        "could not compile",
        "failed to compile",
    ]) || lines
        .clone()
        .any(|line| line.starts_with("fail ") && line.ends_with("[build failed]"))
    {
        Some((
            "failing-test-compile-error",
            "the run failed to compile. Make the test compile before recording the expected assertion failure",
        ))
    } else {
        None
    }
}

fn is_timeout_result(line: &str) -> bool {
    matches!(line, "timed out" | "test timed out" | "test timeout")
        || line.starts_with("timeout [")
        || line.starts_with("error: test timed out")
        || (line.starts_with("test ")
            && line
                .rsplit_once(" timed out after ")
                .is_some_and(|(_, duration)| {
                    duration.starts_with(|character: char| character.is_ascii_digit())
                        && !duration.contains(" ... ")
                }))
}

fn compiler_diagnostic(line: &str) -> &str {
    if let Some((position, diagnostic)) = line.split_once(": error: ")
        && position
            .rsplit(':')
            .next()
            .is_some_and(|line| line.parse::<u32>().is_ok())
    {
        return diagnostic;
    }
    line.strip_prefix("error: ").unwrap_or(line)
}

fn is_assertion_result(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with("assertion failed")
        || line
            .strip_prefix("assertion `")
            .is_some_and(|assertion| assertion.contains("` failed"))
        || line.starts_with("AssertionError:")
        || line.starts_with("assertionerror:")
}
