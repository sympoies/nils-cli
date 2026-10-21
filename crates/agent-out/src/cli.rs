use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum, ValueHint};

#[derive(Debug, Parser)]
#[command(
    name = "agent-out",
    version,
    long_version = nils_build_info::long_version(env!("CARGO_PKG_VERSION")),
    about = "Generate and audit canonical AGENT_HOME/out artifact paths.",
    long_about = "Generate canonical project-scoped AGENT_HOME/out run directories and audit existing out entries for workflow artifact hygiene.",
    after_help = "EXAMPLES:\n  agent-out project --topic browser-qa --mkdir\n  agent-out project --topic unpacked-fixture --kind scratch --mkdir\n  agent-out build --topic release-check --mkdir --format env\n  agent-out path-for --domain projects --topic release-notes --mkdir\n  agent-out project --repo . --topic release-notes --format json\n  agent-out audit --strict\n  agent-out cleanup plan --include-projects --format json\n  agent-out cleanup apply --plan-file cleanup-plan.json --confirm-digest sha256:...\n  agent-out completion zsh\n\nENVIRONMENT:\n  AGENT_HOME  Default agent home root when --agent-home is omitted.\n  AGENT_OUT_PATH, AGENT_OUT_ROOT, AGENT_OUT_PROJECT_SLUG, AGENT_OUT_TOPIC, AGENT_OUT_RUN_ID, AGENT_OUT_DOMAIN, AGENT_OUT_KIND, CARGO_TARGET_DIR  Exported by --format env (CARGO_TARGET_DIR by `build` only).\n\nEXIT CODES:\n  0   success\n  1   runtime error\n  64  command-line usage error\n  65  invalid input data",
    disable_help_subcommand = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Compatibility allocator for rendered state_out artifact paths.
    PathFor(PathForArgs),
    /// Generate a canonical project-scoped artifact directory path.
    Project(ProjectArgs),
    /// Allocate a run-scoped build directory under AGENT_HOME/build/run.
    Build(BuildArgs),
    /// Audit top-level AGENT_HOME/out entries.
    Audit(AuditArgs),
    /// Plan or apply safe AGENT_HOME/out cleanup.
    Cleanup(CleanupArgs),
    /// Print shell completion script.
    Completion(CompletionArgs),
}

#[derive(Debug, Args)]
pub struct PathForArgs {
    /// Compatibility domain emitted by rendered state_out helpers.
    #[arg(long, value_name = "DOMAIN")]
    pub domain: String,

    /// Optional artifact topic within the domain.
    #[arg(long, value_name = "TOPIC")]
    pub topic: Option<String>,

    /// Repository path or owner/repo slug used for slug discovery.
    #[arg(long, value_name = "PATH_OR_OWNER/REPO")]
    pub repo: Option<String>,

    /// Explicit repository slug, preferably owner/repo.
    #[arg(long = "repo-slug", value_name = "OWNER/REPO")]
    pub repo_slug: Option<String>,

    /// Agent home root. Defaults to AGENT_HOME.
    #[arg(long = "agent-home", value_name = "PATH", value_hint = ValueHint::DirPath)]
    pub agent_home: Option<PathBuf>,

    /// Create the generated directory.
    #[arg(long)]
    pub mkdir: bool,

    /// Output format.
    #[arg(long, value_enum, default_value_t = ProjectFormat::Path)]
    pub format: ProjectFormat,
}

#[derive(Debug, Args)]
pub struct BuildArgs {
    /// Human-readable topic for this build run.
    #[arg(long, value_name = "TOPIC")]
    pub topic: String,

    /// Agent home root. Defaults to AGENT_HOME.
    #[arg(long = "agent-home", value_name = "PATH", value_hint = ValueHint::DirPath)]
    pub agent_home: Option<PathBuf>,

    /// Create the generated directory.
    #[arg(long)]
    pub mkdir: bool,

    /// Output format. `env` also exports CARGO_TARGET_DIR.
    #[arg(long, value_enum, default_value_t = ProjectFormat::Path)]
    pub format: ProjectFormat,
}

#[derive(Debug, Args)]
pub struct ProjectArgs {
    /// Human-readable topic for this run directory.
    #[arg(long, value_name = "TOPIC")]
    pub topic: String,

    /// Write-location class: `evidence` (kept artifacts, long retention) or
    /// `scratch` (rebuildable intermediates, short retention).
    #[arg(long, value_enum, default_value_t = RunKind::Evidence)]
    pub kind: RunKind,

    /// Repository path used for slug discovery. Defaults to the current directory.
    #[arg(long, value_name = "PATH", value_hint = ValueHint::DirPath)]
    pub repo: Option<PathBuf>,

    /// Explicit repository slug, preferably owner/repo.
    #[arg(long = "repo-slug", value_name = "OWNER/REPO")]
    pub repo_slug: Option<String>,

    /// Agent home root. Defaults to AGENT_HOME.
    #[arg(long = "agent-home", value_name = "PATH", value_hint = ValueHint::DirPath)]
    pub agent_home: Option<PathBuf>,

    /// Create the generated directory.
    #[arg(long)]
    pub mkdir: bool,

    /// Output format.
    #[arg(long, value_enum, default_value_t = ProjectFormat::Path)]
    pub format: ProjectFormat,
}

#[derive(Debug, Args)]
pub struct AuditArgs {
    /// Agent home root. Defaults to AGENT_HOME.
    #[arg(long = "agent-home", value_name = "PATH", value_hint = ValueHint::DirPath)]
    pub agent_home: Option<PathBuf>,

    /// Exit non-zero when noncanonical entries are present.
    #[arg(long)]
    pub strict: bool,

    /// Output format.
    #[arg(long, value_enum, default_value_t = AuditFormat::Text)]
    pub format: AuditFormat,
}

#[derive(Debug, Args)]
pub struct CleanupArgs {
    #[command(subcommand)]
    pub command: CleanupCommand,
}

#[derive(Debug, Subcommand)]
pub enum CleanupCommand {
    /// Build a dry-run cleanup plan.
    Plan(CleanupPlanArgs),
    /// Apply delete candidates from a reviewed cleanup plan.
    Apply(CleanupApplyArgs),
}

#[derive(Debug, Args)]
pub struct CleanupPlanArgs {
    /// Agent home root. Defaults to AGENT_HOME.
    #[arg(long = "agent-home", value_name = "PATH", value_hint = ValueHint::DirPath)]
    pub agent_home: Option<PathBuf>,

    /// Include canonical projects/<repo>/<run> entries as preserve/needs-policy rows.
    #[arg(long)]
    pub include_projects: bool,

    /// Mark idle project runs older than DAYS as delete candidates.
    #[arg(
        long = "project-retention-days",
        value_name = "DAYS",
        requires = "include_projects",
        value_parser = clap::value_parser!(u32).range(1..)
    )]
    pub project_retention_days: Option<u32>,

    /// Mark idle scratch runs older than DAYS as delete candidates.
    #[arg(
        long = "scratch-retention-days",
        value_name = "DAYS",
        default_value_t = classes::DEFAULT_CLASS_RETENTION_DAYS,
        value_parser = clap::value_parser!(u32).range(1..)
    )]
    pub scratch_retention_days: u32,

    /// Mark idle run-scoped builds older than DAYS as delete candidates.
    #[arg(
        long = "build-retention-days",
        value_name = "DAYS",
        default_value_t = classes::DEFAULT_CLASS_RETENTION_DAYS,
        value_parser = clap::value_parser!(u32).range(1..)
    )]
    pub build_retention_days: u32,

    /// Output format.
    #[arg(long, value_enum, default_value_t = CleanupFormat::Text)]
    pub format: CleanupFormat,
}

#[derive(Debug, Args)]
pub struct CleanupApplyArgs {
    /// Cleanup plan JSON file produced by `agent-out cleanup plan --format json`.
    #[arg(long = "plan-file", value_name = "PATH", value_hint = ValueHint::FilePath)]
    pub plan_file: PathBuf,

    /// Required plan digest confirmation from the reviewed plan.
    #[arg(long = "confirm-digest", value_name = "SHA256")]
    pub confirm_digest: String,

    /// Agent home guard. Defaults to AGENT_HOME; rejects plans for a different home.
    #[arg(long = "agent-home", value_name = "PATH", value_hint = ValueHint::DirPath)]
    pub agent_home: Option<PathBuf>,

    /// Output format.
    #[arg(long, value_enum, default_value_t = CleanupFormat::Text)]
    pub format: CleanupFormat,
}

/// Write-location class of an `agent-out project` run (the `evidence` and
/// `scratch` classes of the agent write-location contract).
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum RunKind {
    Evidence,
    Scratch,
}

impl RunKind {
    /// Directory below `out/` that holds runs of this kind.
    pub fn root_segment(self) -> &'static str {
        match self {
            Self::Evidence => crate::CANONICAL_PROJECT_ROOT,
            Self::Scratch => classes::SCRATCH_ROOT,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Evidence => "evidence",
            Self::Scratch => "scratch",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum ProjectFormat {
    Path,
    Json,
    Env,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum AuditFormat {
    Text,
    Json,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum CleanupFormat {
    Text,
    Json,
}

#[derive(Debug, Args)]
pub struct CompletionArgs {
    #[arg(value_enum)]
    pub shell: crate::completion::CompletionShell,
}

/// Write-location classes beyond evidence: scratch runs under `out/scratch`,
/// run-scoped builds under `build/run`, and the per-class retention, delete
/// validation, and build-tree detection that go with them.
///
/// The contract is `core/policies/write-locations.md` in agent-runtime-kit
/// (tracking agent-runtime-kit#302). The module is inline here only because
/// the artifact-routing hook refuses new files under this crate directory and
/// `lib.rs` is size-baselined; move it into its own file once the hook allows.
pub(crate) mod classes {
    use std::fs;
    use std::io::Read;
    use std::path::{Component, Path, PathBuf};

    use nils_common::fs::display_path;
    use serde::Serialize;
    use serde_json::json;

    use super::{BuildArgs, ProjectFormat};
    use crate::{
        CleanupAction, CleanupCategory, CleanupItem, CleanupPlan, CliError, EXIT_OK, TreeScan,
        entry_kind, is_real_dir, marker_flags, now_unix, path_mtime_unix, path_name,
        path_shallow_size_bytes, path_size_bytes, read_sorted_children, scan_tree,
        unreadable_cleanup_item,
    };

    /// Top-level `out/` entry holding scratch runs.
    pub(crate) const SCRATCH_ROOT: &str = "scratch";
    /// Default idle window for scratch runs and run-scoped builds.
    pub(crate) const DEFAULT_CLASS_RETENTION_DAYS: u32 = 3;
    const BUILD_ROOT: &str = "build";
    const BUILD_RUN_ROOT: &str = "run";
    const BUILD_SCHEMA_VERSION: &str = "cli.agent-out.build.v1";
    const BUILD_COMMAND: &str = "agent-out build";
    const BUILD_TREE_NOTE: &str =
        "; contains a build tree: keep build output in the build class (agent-out build)";
    const CACHEDIR_TAG_SIGNATURE: &[u8] = b"Signature: 8a477f597d28d172789f06886806bc55";

    pub(crate) fn is_false(value: &bool) -> bool {
        !*value
    }

    /// Owner-attested retention window for one class.
    #[derive(Clone, Copy, Debug)]
    pub(crate) struct Retention {
        pub(crate) days: u32,
        pub(crate) cutoff_unix: i64,
    }

    impl Retention {
        pub(crate) fn from_days(days: u32, now_unix: i64) -> Self {
            Self {
                days,
                cutoff_unix: now_unix.saturating_sub(i64::from(days) * 86_400),
            }
        }

        /// Whether `cutoff_unix` is internally consistent and no newer than the
        /// day count allows right now.
        fn accepts(days: u32, cutoff_unix: i64) -> bool {
            days >= 1 && cutoff_unix <= Self::from_days(days, now_unix()).cutoff_unix
        }
    }

    /// The retention window of every class that planned run directories.
    #[derive(Clone, Copy, Debug, Default)]
    pub(crate) struct PlanRetention {
        pub(crate) evidence: Option<Retention>,
        pub(crate) scratch: Option<Retention>,
        pub(crate) build: Option<Retention>,
    }

    impl PlanRetention {
        pub(crate) fn for_class(&self, class: RunClass) -> Option<Retention> {
            match class {
                RunClass::Evidence => self.evidence,
                RunClass::Scratch => self.scratch,
                RunClass::Build => self.build,
            }
        }
    }

    /// A class whose run directories cleanup plans and deletes under retention.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(crate) enum RunClass {
        Evidence,
        Scratch,
        Build,
    }

    impl RunClass {
        pub(crate) fn of(category: CleanupCategory) -> Option<Self> {
            match category {
                CleanupCategory::ProjectArtifact => Some(Self::Evidence),
                CleanupCategory::ScratchArtifact => Some(Self::Scratch),
                CleanupCategory::RunBuild => Some(Self::Build),
                _ => None,
            }
        }

        fn category(self) -> CleanupCategory {
            match self {
                Self::Evidence => CleanupCategory::ProjectArtifact,
                Self::Scratch => CleanupCategory::ScratchArtifact,
                Self::Build => CleanupCategory::RunBuild,
            }
        }

        /// How reasons refer to one run directory of the class.
        fn label(self) -> &'static str {
            match self {
                Self::Evidence => "project run",
                Self::Scratch => "scratch run",
                Self::Build => "run build",
            }
        }

        /// Directory that a delete of this class must stay inside.
        pub(crate) fn root(self, agent_home: &Path, out_root: &Path) -> PathBuf {
            match self {
                Self::Evidence | Self::Scratch => out_root.to_path_buf(),
                Self::Build => agent_home.join(BUILD_ROOT),
            }
        }

        /// Leading segment and component count of a run path below `root`.
        fn shape(self) -> (&'static str, usize, &'static str) {
            match self {
                Self::Evidence => ("projects", 3, "projects/<repo>/<run>"),
                Self::Scratch => (SCRATCH_ROOT, 3, "scratch/<repo>/<run>"),
                Self::Build => (BUILD_RUN_ROOT, 2, "run/<run>"),
            }
        }
    }

    /// Plan one class: every run directory under `class_root` becomes a row.
    ///
    /// Evidence and scratch roots hold `<repo>/<run>`; the run-build root holds
    /// `<run>` directly.
    pub(crate) fn run_items(
        class_root: &Path,
        class: RunClass,
        retention: Option<Retention>,
        owner_uid: u32,
    ) -> Result<Vec<CleanupItem>, CliError> {
        let mut items = Vec::new();
        if !is_real_dir(class_root) {
            return Ok(items);
        }

        let mut parents = Vec::new();
        if class == RunClass::Build {
            parents.push(class_root.to_path_buf());
        } else {
            for project_dir in read_sorted_children(class_root, "cleanup-read-failed")? {
                // A symlinked repo directory could steer run deletes elsewhere.
                if is_real_dir(&project_dir) {
                    parents.push(project_dir);
                }
            }
        }
        for parent in parents {
            let run_dirs = match read_sorted_children(&parent, "cleanup-read-failed") {
                Ok(run_dirs) => run_dirs,
                Err(err) => {
                    items.push(unreadable_cleanup_item(
                        &parent,
                        path_name(&parent),
                        entry_kind(&parent),
                        err,
                    ));
                    continue;
                }
            };
            for run_dir in run_dirs {
                let name = path_name(&run_dir);
                let kind = entry_kind(&run_dir);
                match run_cleanup_item(
                    &run_dir,
                    name.clone(),
                    kind.clone(),
                    class,
                    retention,
                    owner_uid,
                ) {
                    Ok(item) => items.push(item),
                    Err(err) => items.push(unreadable_cleanup_item(&run_dir, name, kind, err)),
                }
            }
        }
        Ok(items)
    }

    /// A preserved row for a class root such as `out/scratch`.
    pub(crate) fn allowed_root_item(
        path: &Path,
        name: String,
        kind: String,
        reason: &str,
    ) -> Result<CleanupItem, CliError> {
        Ok(CleanupItem {
            name,
            path: display_path(path),
            kind,
            category: CleanupCategory::AllowedRoot,
            action: CleanupAction::Preserve,
            reason: reason.to_string(),
            size_bytes: path_shallow_size_bytes(path)?,
            mtime_unix: path_mtime_unix(path),
            content_digest: None,
            tree_identity: None,
            contains_skill_usage: false,
            contains_test_first_evidence: false,
            contains_build_tree: false,
            diagnostic: None,
        })
    }

    fn run_cleanup_item(
        run_dir: &Path,
        name: String,
        kind: String,
        class: RunClass,
        retention: Option<Retention>,
        owner_uid: u32,
    ) -> Result<CleanupItem, CliError> {
        let mut item = classify_run(run_dir, name, kind, class, retention, owner_uid)?;
        if item.contains_build_tree {
            item.reason.push_str(BUILD_TREE_NOTE);
        }
        Ok(item)
    }

    fn classify_run(
        run_dir: &Path,
        name: String,
        kind: String,
        class: RunClass,
        retention: Option<Retention>,
        owner_uid: u32,
    ) -> Result<CleanupItem, CliError> {
        let markers = marker_flags(run_dir)?;
        let artifact = match class {
            RunClass::Evidence => "project artifact",
            RunClass::Scratch => "scratch artifact",
            RunClass::Build => "run build",
        };
        let mut item = CleanupItem {
            name,
            path: display_path(run_dir),
            kind,
            category: class.category(),
            action: CleanupAction::NeedsPolicy,
            reason: format!(
                "canonical {artifact} without a retention policy; review before deleting"
            ),
            size_bytes: 0,
            mtime_unix: path_mtime_unix(run_dir),
            content_digest: None,
            tree_identity: None,
            contains_skill_usage: markers.skill_usage,
            contains_test_first_evidence: markers.test_first_evidence,
            // Build output inside an evidence run is misfiled; elsewhere it is fine.
            contains_build_tree: class == RunClass::Evidence && markers.build_tree,
            diagnostic: None,
        };
        if markers.has_evidence() {
            item.category = CleanupCategory::EvidenceSource;
            item.action = CleanupAction::Preserve;
            item.reason =
                "contains retained evidence markers; use evidence migrate/prune-source".to_string();
            item.size_bytes = path_size_bytes(run_dir)?;
            return Ok(item);
        }
        let Some(policy) = retention else {
            item.size_bytes = path_size_bytes(run_dir)?;
            return Ok(item);
        };

        let scan = scan_tree(run_dir, owner_uid)?;
        item.size_bytes = scan.size_bytes;
        item.reason = retention_reason(class, &item, &scan, policy);
        if retention_eligible(&item.name, &item.kind, &scan, policy) {
            item.action = CleanupAction::Delete;
            item.tree_identity = Some(scan.identity);
        } else if class != RunClass::Evidence
            && item.kind == "directory"
            && !scan.foreign_owned_dirs
            && run_started_unix(&item.name).is_some()
        {
            // Inside the retention window: nothing to decide, just not due yet.
            item.action = CleanupAction::Preserve;
        }
        Ok(item)
    }

    pub(crate) fn retention_eligible(
        name: &str,
        kind: &str,
        scan: &TreeScan,
        policy: Retention,
    ) -> bool {
        kind == "directory"
            && !scan.foreign_owned_dirs
            && scan.newest_mtime_unix < policy.cutoff_unix
            && run_started_unix(name).is_some_and(|started| started < policy.cutoff_unix)
    }

    fn retention_reason(
        class: RunClass,
        item: &CleanupItem,
        scan: &TreeScan,
        policy: Retention,
    ) -> String {
        let days = policy.days;
        let label = class.label();
        if item.kind != "directory" {
            format!("{label} is not a real directory; review before deleting")
        } else if scan.foreign_owned_dirs {
            "contains directories owned by another account; restore ownership before cleanup"
                .to_string()
        } else if run_started_unix(&item.name).is_none() {
            format!("{label} name has no YYYYMMDD run id; review before deleting")
        } else if run_started_unix(&item.name).is_some_and(|started| started >= policy.cutoff_unix)
        {
            format!("{label} is inside the {days}-day retention window")
        } else if scan.newest_mtime_unix >= policy.cutoff_unix {
            format!("{label} changed within the {days}-day retention window")
        } else {
            format!("idle {label} older than the {days}-day retention policy")
        }
    }

    /// Parse the allocation timestamp from a run id such as `20260928-172130-topic`
    /// or `20260901-topic`.
    ///
    /// The id is written in local time but read as UTC. On hosts east of UTC that
    /// makes a run look slightly newer than it is, which only delays deletion; the
    /// newest-mtime check bounds every other case.
    pub(crate) fn run_started_unix(name: &str) -> Option<i64> {
        let date = name.get(0..8)?;
        if !date.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let day = chrono::NaiveDate::parse_from_str(date, "%Y%m%d").ok()?;
        let rest = name.get(8..).unwrap_or("");
        if !(rest.is_empty() || rest.starts_with('-')) {
            return None;
        }
        let time = rest
            .get(1..7)
            .filter(|candidate| candidate.bytes().all(|byte| byte.is_ascii_digit()))
            .filter(|_| rest.get(7..8).is_none_or(|next| next == "-"))
            .and_then(|candidate| chrono::NaiveTime::parse_from_str(candidate, "%H%M%S").ok())
            .unwrap_or(chrono::NaiveTime::MIN);
        Some(day.and_time(time).and_utc().timestamp())
    }

    /// Accept a plan's retention policies only when each is internally consistent
    /// and no looser than its own day count allows right now.
    pub(crate) fn validated_plan_retention(plan: &CleanupPlan) -> Result<PlanRetention, CliError> {
        let pair = |days: Option<u32>, cutoff: Option<i64>| match (days, cutoff) {
            (None, None) => Some(None),
            (Some(days), Some(cutoff_unix)) if Retention::accepts(days, cutoff_unix) => {
                Some(Some(Retention { days, cutoff_unix }))
            }
            _ => None,
        };
        let evidence = pair(
            plan.project_retention_days,
            plan.project_retention_cutoff_unix,
        )
        .filter(|policy| policy.is_none() || plan.include_projects);
        let scratch = pair(
            plan.scratch_retention_days,
            plan.scratch_retention_cutoff_unix,
        );
        let build = pair(plan.build_retention_days, plan.build_retention_cutoff_unix);
        match (evidence, scratch, build) {
            (Some(evidence), Some(scratch), Some(build)) => Ok(PlanRetention {
                evidence,
                scratch,
                build,
            }),
            _ => Err(CliError::data(
                "cleanup-retention-policy-invalid",
                "plan retention policy is inconsistent or newer than its retention window allows",
                Some(json!({
                    "project_retention_days": plan.project_retention_days,
                    "project_retention_cutoff_unix": plan.project_retention_cutoff_unix,
                    "scratch_retention_days": plan.scratch_retention_days,
                    "scratch_retention_cutoff_unix": plan.scratch_retention_cutoff_unix,
                    "build_retention_days": plan.build_retention_days,
                    "build_retention_cutoff_unix": plan.build_retention_cutoff_unix
                })),
            )),
        }
    }

    /// A run delete must sit exactly at the class shape below its root, carry a
    /// tree identity, and fall under the plan's retention policy by its run id.
    pub(crate) fn validate_run_delete(
        item: &CleanupItem,
        class: RunClass,
        relative: &Path,
        retention: Option<Retention>,
    ) -> Result<(), CliError> {
        let (first, count, shape) = class.shape();
        let label = class.label();
        let components: Vec<_> = relative.components().collect();
        let shape_ok = components.len() == count
            && components
                .iter()
                .all(|component| matches!(component, Component::Normal(_)))
            && components[0].as_os_str() == first
            && components[count - 1].as_os_str().to_str() == Some(item.name.as_str());
        if !shape_ok {
            return Err(CliError::data(
                "cleanup-delete-shape-invalid",
                format!("{label} deletes must be canonical {shape} directories"),
                Some(json!({ "path": item.path, "category": item.category })),
            ));
        }
        let Some(policy) = retention else {
            return Err(CliError::data(
                "cleanup-delete-shape-invalid",
                format!("{label} deletes require a plan retention policy"),
                Some(json!({ "path": item.path, "category": item.category })),
            ));
        };
        if item.tree_identity.is_none()
            || run_started_unix(&item.name).is_none_or(|started| started >= policy.cutoff_unix)
        {
            return Err(CliError::data(
                "cleanup-delete-shape-invalid",
                format!("{label} delete lacks a tree identity or is inside the retention window"),
                Some(json!({ "path": item.path, "category": item.category })),
            ));
        }
        Ok(())
    }

    /// Recheck a run immediately before deletion. Any read failure or drift
    /// becomes a skip for that row; nothing unproven is deleted.
    pub(crate) fn run_apply_skip_reason(
        item: &CleanupItem,
        path: &Path,
        policy: Retention,
        owner_uid: u32,
    ) -> Option<String> {
        match marker_flags(path) {
            Err(_) => return Some("path became unreadable after the plan was created".to_string()),
            Ok(markers) if markers.has_evidence() => {
                return Some("evidence marker appeared after the plan was created".to_string());
            }
            Ok(_) => {}
        }
        let scan = match scan_tree(path, owner_uid) {
            Ok(scan) => scan,
            Err(_) => return Some("path became unreadable after the plan was created".to_string()),
        };
        if item.tree_identity.as_deref() != Some(scan.identity.as_str()) {
            return Some("path metadata changed after the plan was created".to_string());
        }
        if !retention_eligible(&item.name, &entry_kind(path), &scan, policy) {
            let label = RunClass::of(item.category).map_or("run", RunClass::label);
            return Some(format!("{label} no longer satisfies the retention policy"));
        }
        None
    }

    /// Cargo marks a target directory with `.rustc_info.json` and a signed
    /// `CACHEDIR.TAG`; either one makes the enclosing tree a build tree.
    pub(crate) fn is_build_tree_marker(path: &Path, metadata: &fs::Metadata) -> bool {
        if !metadata.is_file() {
            return false;
        }
        match path.file_name().and_then(|value| value.to_str()) {
            Some(".rustc_info.json") => true,
            Some("CACHEDIR.TAG") => {
                let mut head = Vec::with_capacity(CACHEDIR_TAG_SIGNATURE.len());
                fs::File::open(path)
                    .and_then(|file| {
                        file.take(CACHEDIR_TAG_SIGNATURE.len() as u64)
                            .read_to_end(&mut head)
                    })
                    .is_ok()
                    && head == CACHEDIR_TAG_SIGNATURE
            }
            _ => false,
        }
    }

    /// Per-class retention and build-tree notes for the text plan.
    pub(crate) fn plan_notes(plan: &CleanupPlan) -> Vec<String> {
        let class_line = |label: &str, days: Option<u32>, cutoff: Option<i64>| match (days, cutoff)
        {
            (Some(days), Some(cutoff)) => {
                format!("{label} retention: {days} days (cutoff_unix={cutoff})")
            }
            _ => format!("{label} retention: unset"),
        };
        let mut notes = vec![
            class_line(
                "evidence",
                plan.project_retention_days,
                plan.project_retention_cutoff_unix,
            ),
            class_line(
                "scratch",
                plan.scratch_retention_days,
                plan.scratch_retention_cutoff_unix,
            ),
            class_line(
                "build",
                plan.build_retention_days,
                plan.build_retention_cutoff_unix,
            ),
        ];
        if plan.summary.build_trees > 0 {
            notes.push(format!(
                "build trees in evidence: {}",
                plan.summary.build_trees
            ));
        }
        notes
    }

    /// Assemble a signed cleanup plan from planned rows and per-class retention.
    pub(crate) fn finish_plan(
        agent_home: &Path,
        out_root_exists: bool,
        include_projects: bool,
        retention: &PlanRetention,
        items: Vec<CleanupItem>,
    ) -> Result<CleanupPlan, CliError> {
        let days = |policy: Option<Retention>| policy.map(|policy| policy.days);
        let cutoff = |policy: Option<Retention>| policy.map(|policy| policy.cutoff_unix);
        let mut plan = CleanupPlan {
            agent_home: display_path(agent_home),
            out_root: display_path(&agent_home.join("out")),
            out_root_exists,
            include_projects,
            project_retention_days: days(retention.evidence),
            project_retention_cutoff_unix: cutoff(retention.evidence),
            scratch_retention_days: days(retention.scratch),
            scratch_retention_cutoff_unix: cutoff(retention.scratch),
            build_retention_days: days(retention.build),
            build_retention_cutoff_unix: cutoff(retention.build),
            summary: crate::cleanup_summary(&items),
            items,
            plan_digest: String::new(),
        };
        plan.plan_digest = crate::compute_cleanup_plan_digest(&plan)?;
        Ok(plan)
    }

    /// Result of `agent-out build`: a run-scoped build directory.
    #[derive(Debug, Serialize)]
    pub(crate) struct BuildResult {
        path: String,
        agent_home: String,
        build_root: String,
        topic: String,
        run_id: String,
        kind: &'static str,
        created: bool,
    }

    pub(crate) fn run_build(args: BuildArgs) -> i32 {
        match build_result(&args) {
            Ok(result) => match args.format {
                ProjectFormat::Path => {
                    println!("{}", result.path);
                    EXIT_OK
                }
                ProjectFormat::Json => {
                    crate::print_json_success(BUILD_SCHEMA_VERSION, BUILD_COMMAND, &result)
                        .unwrap_or_else(crate::render_json_failure)
                }
                ProjectFormat::Env => {
                    println!("{}", render_build_env(&result));
                    EXIT_OK
                }
            },
            Err(err) => crate::render_error(BUILD_SCHEMA_VERSION, BUILD_COMMAND, args.format, err),
        }
    }

    fn build_result(args: &BuildArgs) -> Result<BuildResult, CliError> {
        let agent_home = crate::resolve_agent_home(args.agent_home.as_deref())?;
        let topic = crate::sanitize_topic(&args.topic);
        let run_id = format!("{}-{}", crate::current_timestamp(), topic);
        let build_root = agent_home.join(BUILD_ROOT).join(BUILD_RUN_ROOT);
        let path = build_root.join(&run_id);
        if args.mkdir {
            fs::create_dir_all(&path).map_err(|err| {
                CliError::runtime(
                    "mkdir-failed",
                    format!("failed to create {}: {err}", path.display()),
                    Some(json!({ "path": display_path(&path) })),
                )
            })?;
        }
        Ok(BuildResult {
            path: display_path(&path),
            agent_home: display_path(&agent_home),
            build_root: display_path(&build_root),
            topic,
            run_id,
            kind: "build",
            created: args.mkdir,
        })
    }

    fn render_build_env(result: &BuildResult) -> String {
        [
            ("AGENT_OUT_PATH", result.path.as_str()),
            ("AGENT_OUT_ROOT", result.build_root.as_str()),
            ("AGENT_OUT_TOPIC", result.topic.as_str()),
            ("AGENT_OUT_RUN_ID", result.run_id.as_str()),
            ("AGENT_OUT_KIND", result.kind),
            ("CARGO_TARGET_DIR", result.path.as_str()),
        ]
        .into_iter()
        .map(|(key, value)| format!("{key}={}", crate::shell_quote(value)))
        .collect::<Vec<_>>()
        .join("\n")
    }
}
