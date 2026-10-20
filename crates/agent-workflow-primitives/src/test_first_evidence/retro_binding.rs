use super::*;

#[derive(Debug, clap::Args)]
pub struct RetroBindArgs {
    #[command(flatten)]
    pub subject: SubjectArgs,
    /// Full commit object ID of the original pre-edit baseline (never inferred from HEAD).
    #[arg(long)]
    pub baseline: String,
    /// Retained log from the original failing run.
    #[arg(long, value_hint = clap::ValueHint::FilePath)]
    pub red_log: PathBuf,
    /// Retained log from the passing focused run after implementation.
    #[arg(long, value_hint = clap::ValueHint::FilePath)]
    pub green_log: PathBuf,
    /// Command that produced the retained RED log.
    #[arg(long)]
    pub red_command: String,
    /// Command that produced the retained GREEN log.
    #[arg(long)]
    pub green_command: String,
    /// Actual nonzero exit status from the original RED run.
    #[arg(long, allow_hyphen_values = true)]
    pub red_exit_code: i32,
    /// Actual exit status from the GREEN run (must be zero).
    #[arg(long, allow_hyphen_values = true)]
    pub green_exit_code: i32,
    /// Intended contract failure, excluding setup or compilation errors.
    #[arg(long)]
    pub expected_failure: String,
    /// Meaningful failure observed in the retained RED log.
    #[arg(long)]
    pub observed_failure: String,
    /// Why the original evidence was retained without initializing a record.
    #[arg(long)]
    pub reason: String,
    /// Attest that RED preceded production edits and GREEN followed the implementation.
    #[arg(long)]
    pub attest_pre_edit_red: bool,
}

/// Explicit operator attestation; hashes preserve the imported bytes, not proof of chronology.
#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct RetroBinding {
    pub mode: String,
    pub reason: String,
    pub pre_edit_red_attested: bool,
    pub bound_at: String,
    pub bound_head: String,
    pub red_log_digest: String,
    pub green_log_digest: String,
}

pub(super) fn run(args: RetroBindArgs) -> i32 {
    const SCHEMA: &str = "cli.test-first-evidence.retro-bind.v2";
    const COMMAND: &str = "test-first-evidence retro-bind";
    let result = update_record(&args.subject.common.out_dir, |record| {
        if record.subject.is_some()
            || !record.failing_tests.is_empty()
            || !record.final_validations.is_empty()
            || record.waiver.is_some()
        {
            return Err(invalid(
                "retro-binding-already-recorded",
                "retro-bind requires an unbound v2 record without RED, GREEN, or waiver evidence",
            ));
        }
        if !args.attest_pre_edit_red || args.red_exit_code == 0 || args.green_exit_code != 0 {
            return Err(invalid(
                "invalid-retained-evidence",
                "attest pre-edit RED, provide a nonzero RED exit code, and a zero GREEN exit code",
            ));
        }
        let reason = text(&args.reason, "--reason")?;
        let expected_failure = text(&args.expected_failure, "--expected-failure")?;
        let observed_failure = text(&args.observed_failure, "--observed-failure")?;
        let red_command = text(&args.red_command, "--red-command")?;
        let green_command = text(&args.green_command, "--green-command")?;
        let project = &args.subject.project_path;
        let baseline = git_output(
            project,
            &[
                "rev-parse",
                "--verify",
                "--end-of-options",
                &format!("{}^{{commit}}", args.baseline),
            ],
            "retained baseline",
        )?;
        if !canonical_oid(&args.baseline, baseline.len()) || baseline != args.baseline {
            return Err(invalid(
                "invalid-retained-baseline",
                "--baseline must name the original full commit object ID",
            ));
        }
        if git_output(
            project,
            &["merge-base", &baseline, "HEAD"],
            "retained baseline ancestry",
        )? != baseline
        {
            return Err(invalid(
                "invalid-retained-baseline",
                "the retained baseline must be an ancestor of HEAD",
            ));
        }
        let tree = git_output(
            project,
            &["rev-parse", "--verify", &format!("{baseline}^{{tree}}")],
            "retained baseline tree",
        )?;
        let red_log_digest = log_digest(&args.red_log)?;
        let green_log_digest = log_digest(&args.green_log)?;
        let subject = EvidenceSubject {
            repository: repository_identity(
                project,
                &args.subject.remote,
                args.subject.repository_id.as_deref(),
            )?,
            baseline: BaselineSubject {
                commit: baseline,
                tree,
            },
            deliveries: Vec::new(),
            retro_binding: Some(RetroBinding {
                mode: "retro-bound".to_string(),
                reason: reason.clone(),
                pre_edit_red_attested: true,
                bound_at: jiff::Timestamp::now().to_string(),
                bound_head: git_output(project, &["rev-parse", "HEAD^{commit}"], "import head")?,
                red_log_digest,
                green_log_digest,
            }),
        };
        record.failing_tests.push(FailingEvidence {
            command: red_command,
            exit_code: args.red_exit_code,
            summary: reason,
            expected_failure,
            observed_failure,
            test_name: None,
            artifacts: normalized_paths(std::slice::from_ref(&args.red_log)),
        });
        record.final_validations.push(FinalValidation {
            command: green_command,
            status: ValidationStatus::Pass,
            scope: Some(ValidationScope::Focused),
            attempt: 1,
            resolution: None,
            summary: Some(
                "Imported retained focused GREEN; see retro-binding provenance".to_string(),
            ),
            artifacts: normalized_paths(std::slice::from_ref(&args.green_log)),
        });
        record.subject = Some(subject);
        Ok(())
    });
    match result {
        Ok(result) => render_record_success(SCHEMA, COMMAND, args.subject.common.format, &result),
        Err(err) => render_error(SCHEMA, COMMAND, args.subject.common.format, err),
    }
}

fn invalid(code: &'static str, message: &str) -> CliError {
    CliError::data(code, message, None)
}

fn text(value: &str, flag: &str) -> Result<String, CliError> {
    if value.trim().is_empty() {
        return Err(invalid(
            "invalid-retained-evidence",
            &format!("{flag} must not be blank"),
        ));
    }
    Ok(redact_text(value.trim()).value)
}

fn log_digest(path: &Path) -> Result<String, CliError> {
    let bytes = fs::read(path).map_err(|_| {
        invalid(
            "retained-log-unreadable",
            "retained logs must be readable files",
        )
    })?;
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Err(invalid(
            "retained-log-empty",
            "retained logs must contain nonblank evidence",
        ));
    }
    Ok(format!("sha256:{}", hex(&Sha256::digest(bytes))))
}

pub(super) fn valid_provenance(record: &EvidenceRecord) -> bool {
    let Some(subject) = &record.subject else {
        return true;
    };
    let Some(binding) = &subject.retro_binding else {
        return true;
    };
    binding.mode == "retro-bound"
        && binding.pre_edit_red_attested
        && !binding.reason.trim().is_empty()
        && binding.bound_at.parse::<jiff::Timestamp>().is_ok()
        && canonical_oid(&binding.bound_head, subject.baseline.commit.len())
        && canonical_sha256_digest(&binding.red_log_digest)
        && canonical_sha256_digest(&binding.green_log_digest)
        && !record.failing_tests.is_empty()
        && record.final_validations.iter().any(|item| {
            item.status == ValidationStatus::Pass && item.scope == Some(ValidationScope::Focused)
        })
}

pub(super) fn print_provenance(record: &EvidenceRecord) {
    if let Some(binding) = record
        .subject
        .as_ref()
        .and_then(|subject| subject.retro_binding.as_ref())
    {
        println!(
            "evidence binding: {} (caller-attested chronology)",
            binding.mode
        );
    }
}
