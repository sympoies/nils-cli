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

// The caller supplies the observed diagnostic. Match runner/compiler diagnostics
// at the start of a line so an assertion can refer to a timeout or missing symbol.
pub(super) fn invalid_failure(
    exit_code: i32,
    observed: &str,
) -> Option<(&'static str, &'static str)> {
    let observed = observed.to_ascii_lowercase();
    let has_diagnostic = |prefixes: &[&str]| {
        observed
            .lines()
            .map(str::trim_start)
            .any(|line| prefixes.iter().any(|prefix| line.starts_with(prefix)))
    };
    if exit_code == 124
        || has_diagnostic(&["timeout", "timed out", "error: test timed out"])
        || observed.lines().map(str::trim_start).any(|line| {
            line.starts_with("test ")
                && (line.contains(" timed out") || line.starts_with("test timeout"))
        })
    {
        Some((
            "failing-test-timeout",
            "the run timed out. Record the expected assertion failure after the test runs",
        ))
    } else if has_diagnostic(&[
        "missing symbol",
        "undefined symbol",
        "undefined reference",
        "cannot find symbol",
        "error: missing symbol",
        "error: undefined symbol",
        "error: cannot find symbol",
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
        "error: could not compile",
        "error: failed to compile",
    ]) {
        Some((
            "failing-test-compile-error",
            "the run failed to compile. Make the test compile before recording the expected assertion failure",
        ))
    } else {
        None
    }
}
