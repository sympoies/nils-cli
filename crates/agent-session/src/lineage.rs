//! Session lineage and work references (`session-lineage-work-v1`).
//!
//! `lineage` records which session started this one. It is written once when
//! the record is created and is descriptive only: it never authorizes
//! anything. `work` names the program and issues a session works on; a child
//! inherits its parent's by default.

use std::collections::BTreeMap;

use nils_common::forge_identity::session::{CONTEXT_KEY, LaunchContext};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use std::path::Path;

use crate::{CliContext, CliError, SessionRecord};

pub(crate) const LINEAGE_SCHEMA: &str = "agent-session.session-lineage.v1";
/// The deepest chain a record may claim. A deeper start fails instead of
/// storing a depth readers cannot trust.
pub(crate) const MAX_DEPTH: u32 = 64;
pub(crate) const MAX_ISSUES: usize = 4;
const MAX_MACHINE_BYTES: usize = 64;
const MAX_INCARNATION_BYTES: usize = 128;
const MAX_TIMESTAMP_BYTES: usize = 64;

pub(crate) const STARTER_SESSION: &str = "session";
pub(crate) const STARTER_CONSOLE: &str = "console";
pub(crate) const STARTER_OPERATOR: &str = "operator";
pub(crate) const STARTER_MAIN_AGENT: &str = "main-agent";
pub(crate) const VIA_CLI: &str = "cli";
pub(crate) const VIA_CONSOLE: &str = "console";
pub(crate) const VIA_HTTP: &str = "http";

/// The coordinator role retains its root-only topology requirement.
pub(crate) const ROLE_COORDINATOR: &str = "coordinator";
const ROLE_INVALID: &str = "role-invalid";
const ROLE_REQUIRES_ROOT: &str = "role-requires-root";
const LINEAGE_INVALID: &str = "lineage-invalid";
const LINEAGE_DEPTH_EXCEEDED: &str = "lineage-depth-exceeded";
const WORK_REF_INVALID: &str = "work-ref-invalid";
const LINEAGE_ADOPT_FORBIDDEN: &str = "lineage-adopt-forbidden";
const LINEAGE_REVISION_CONFLICT: &str = "lineage-revision-conflict";
const WORK_SET_FORBIDDEN: &str = "work-set-forbidden";
const WORK_REVISION_CONFLICT: &str = "work-revision-conflict";
const SESSION_HAS_LIVE_CHILDREN: &str = "session-has-live-children";
pub(crate) const LINEAGE_ADOPT_COMMAND: &str = "lineage-adopt";
pub(crate) const WORK_SET_COMMAND: &str = "work-set";

/// One session, as `(machine, session_id, session_created_at)`. The
/// incarnation is kept on a parent for audit only; it never takes part in a
/// match, so a parent that restarts keeps its children.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SessionRef {
    pub machine: String,
    pub session_id: String,
    pub session_created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_incarnation: Option<String>,
}

impl SessionRef {
    fn of(machine: &str, record: &SessionRecord) -> Self {
        Self {
            machine: machine.to_string(),
            session_id: record.id.clone(),
            session_created_at: record.created_at.clone(),
            session_incarnation: launch_id(record).map(str::to_string),
        }
    }

    fn without_incarnation(&self) -> Self {
        Self {
            session_incarnation: None,
            ..self.clone()
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Starter {
    pub kind: String,
    pub via: String,
}

/// The stored `lineage` object of `agent-session.session.v1`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct SessionLineage {
    pub schema_version: String,
    /// The label of the machine this session runs on, as the process that
    /// created it resolved it. A child started inside this session reuses it.
    #[serde(default)]
    pub machine: String,
    pub parent: Option<SessionRef>,
    pub root: SessionRef,
    pub depth: u32,
    pub starter: Starter,
    /// Reserved for the subtree budget of `session-admission-v1`.
    #[serde(default)]
    pub budget: Option<Value>,
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// One public provider reference: `[provider:]owner/repo#N`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct WorkRef {
    pub provider: String,
    pub repository: String,
    pub number: u64,
}

impl WorkRef {
    pub(crate) fn parse(value: &str) -> Result<Self, CliError> {
        let invalid = || {
            CliError::usage(
                WORK_REF_INVALID,
                "a work reference must be owner/repo#N, optionally prefixed by github: or gitlab:",
                Some(json!({ "reference": bounded(value) })),
            )
        };
        if value.is_empty() || !value.chars().all(|ch| ch.is_ascii_graphic()) {
            return Err(invalid());
        }
        let (provider, rest) = match value.split_once(':') {
            Some((provider, rest)) => (provider, rest),
            None => ("github", value),
        };
        let (repository, number) = rest.split_once('#').ok_or_else(invalid)?;
        if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(invalid());
        }
        let number = number.parse::<u64>().map_err(|_| invalid())?;
        Self {
            provider: provider.to_string(),
            repository: repository.to_string(),
            number,
        }
        .canonical()
        .map_err(|_| invalid())
    }

    /// The reference as `[gitlab:]owner/repo#N`, the grammar `parse` accepts;
    /// GitHub, the default provider, has no prefix.
    pub(crate) fn display(&self) -> String {
        let prefix = if self.provider == "github" {
            String::new()
        } else {
            format!("{}:", self.provider)
        };
        format!("{prefix}{}#{}", self.repository, self.number)
    }

    fn canonical(self) -> Result<Self, CliError> {
        let provider = self.provider.trim().to_ascii_lowercase();
        if provider != "github" && provider != "gitlab" {
            return Err(work_invalid(
                "work reference provider must be github or gitlab",
            ));
        }
        if self.number == 0 {
            return Err(work_invalid("work reference number must be positive"));
        }
        let repository = crate::coordination::context::canonical_repository(self.repository)
            .map_err(|_| work_invalid("work reference repository must be owner/name"))?;
        Ok(Self {
            provider,
            repository,
            number: self.number,
        })
    }
}

/// The stored `work` object of `agent-session.session.v1`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct SessionWork {
    pub program: Option<WorkRef>,
    #[serde(default)]
    pub issues: Vec<WorkRef>,
    #[serde(default)]
    pub inherited: bool,
    #[serde(default)]
    pub revision: u64,
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// What a start asked for explicitly. `None` leaves that dimension to
/// inheritance; `inherit: false` drops whatever was not given.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkRequest {
    pub program: Option<WorkRef>,
    pub issues: Option<Vec<WorkRef>>,
    pub inherit: bool,
}

impl WorkRequest {
    /// From `start --program/--issue/--no-inherit-work`.
    pub(crate) fn from_flags(
        program: Option<&str>,
        issues: &[String],
        no_inherit: bool,
    ) -> Result<Self, CliError> {
        let program = program.map(WorkRef::parse).transpose()?;
        let issues = if issues.is_empty() {
            None
        } else {
            Some(canonical_issues(
                issues
                    .iter()
                    .map(|issue| WorkRef::parse(issue))
                    .collect::<Result<Vec<_>, _>>()?,
            )?)
        };
        Ok(Self {
            program,
            issues,
            inherit: !no_inherit,
        })
    }

    /// The resolved `work` of a new session whose parent has `parent`.
    pub(crate) fn resolve(&self, parent: Option<&SessionWork>) -> Option<SessionWork> {
        let inherited_from = parent.filter(|_| self.inherit);
        let program = self
            .program
            .clone()
            .or_else(|| inherited_from.and_then(|work| work.program.clone()));
        let issues = self.issues.clone().unwrap_or_else(|| {
            inherited_from
                .map(|work| work.issues.clone())
                .unwrap_or_default()
        });
        if program.is_none() && issues.is_empty() {
            return None;
        }
        Some(SessionWork {
            program,
            issues,
            inherited: self.program.is_none() && self.issues.is_none(),
            revision: 1,
            extra: BTreeMap::new(),
        })
    }

    /// The `work` member of a console start request.
    pub(crate) fn to_request_json(&self) -> Option<Value> {
        (self.program.is_some() || self.issues.is_some() || !self.inherit).then(|| {
            let mut value = json!({ "inherit": self.inherit });
            if let Some(program) = &self.program {
                value["program"] = json!(program);
            }
            if let Some(issues) = &self.issues {
                value["issues"] = json!(issues);
            }
            value
        })
    }

    /// The `work` member of a console start request, validated.
    pub(crate) fn from_request_json(value: &Value) -> Result<Self, CliError> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Input {
            #[serde(default)]
            program: Option<WorkRef>,
            #[serde(default)]
            issues: Option<Vec<WorkRef>>,
            #[serde(default = "inherit_default")]
            inherit: bool,
        }
        fn inherit_default() -> bool {
            true
        }
        let input: Input = serde_json::from_value(value.clone())
            .map_err(|_| work_invalid("work must be {program, issues, inherit}"))?;
        Ok(Self {
            program: input.program.map(WorkRef::canonical).transpose()?,
            issues: input.issues.map(canonical_issues).transpose()?,
            inherit: input.inherit,
        })
    }
}

fn canonical_issues(issues: Vec<WorkRef>) -> Result<Vec<WorkRef>, CliError> {
    let mut issues = issues
        .into_iter()
        .map(WorkRef::canonical)
        .collect::<Result<Vec<_>, _>>()?;
    issues.sort();
    issues.dedup();
    if issues.len() > MAX_ISSUES {
        return Err(work_invalid("a session names at most 4 issues"));
    }
    Ok(issues)
}

/// Validate the `role` of a create body or console start. `null` is no role.
pub(crate) fn role_from_request(value: Option<&str>) -> Result<Option<String>, CliError> {
    match value {
        None => Ok(None),
        Some(role)
            if LaunchContext {
                initiator: "operator".into(),
                role: Some(role.into()),
            }
            .validate()
            .is_ok() =>
        {
            Ok(Some(role.into()))
        }
        Some(_) => Err(CliError::usage(
            ROLE_INVALID,
            "role must be a bounded identifier",
            None,
        )),
    }
}

/// Validate the raw `role` of a create body: absent or `null` is no role, and
/// non-identifier values are `role-invalid`.
pub(crate) fn role_from_create_body(value: Option<&Value>) -> Result<Option<String>, CliError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(value) => role_from_request(Some(value.as_str().unwrap_or_default())),
    }
}

/// A coordinator is a root: it may not be started with a parent.
pub(crate) fn require_root_for_role(role: Option<&str>, has_parent: bool) -> Result<(), CliError> {
    if let Some(role) = role
        && matches!(role, ROLE_COORDINATOR | "domain-coordinator")
        && has_parent
    {
        return Err(CliError::usage(
            ROLE_REQUIRES_ROOT,
            format!("role {role} needs a root start: use --no-parent, or a start with no parent"),
            None,
        ));
    }
    Ok(())
}

/// Validate a `work` object a create body supplies. An empty one is no work.
pub(crate) fn work_from_create_body(value: &Value) -> Result<Option<SessionWork>, CliError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input {
        #[serde(default)]
        program: Option<WorkRef>,
        #[serde(default)]
        issues: Vec<WorkRef>,
        #[serde(default)]
        inherited: bool,
    }
    let input: Input = serde_json::from_value(value.clone())
        .map_err(|_| work_invalid("work must be {program, issues, inherited}"))?;
    let program = input.program.map(WorkRef::canonical).transpose()?;
    let issues = canonical_issues(input.issues)?;
    if program.is_none() && issues.is_empty() {
        return Ok(None);
    }
    Ok(Some(SessionWork {
        program,
        issues,
        inherited: input.inherited,
        revision: 1,
        extra: BTreeMap::new(),
    }))
}

/// Everything about a new session's lineage that is known before its id and
/// creation time are.
#[derive(Clone, Debug, PartialEq)]
pub struct LineageSeed {
    /// The label of the machine the new session runs on.
    machine: String,
    parent: Option<ParentLink>,
    starter: Starter,
    forge_context: Option<LaunchContext>,
}

#[derive(Clone, Debug, PartialEq)]
struct ParentLink {
    parent: SessionRef,
    root: SessionRef,
    depth: u32,
}

impl LineageSeed {
    /// A new root: no parent, and the session is its own root.
    pub(crate) fn root(machine: &str, kind: &str, via: &str) -> Self {
        Self {
            machine: machine.to_string(),
            parent: None,
            forge_context: None,
            starter: Starter {
                kind: kind.to_string(),
                via: via.to_string(),
            },
        }
    }

    /// Whether the new session has a parent.
    pub(crate) fn has_parent(&self) -> bool {
        self.parent.is_some()
    }

    /// A child of `parent`, which runs on `parent_machine`.
    pub(crate) fn child_of(
        machine: &str,
        parent_machine: &str,
        parent: &SessionRecord,
        kind: &str,
        via: &str,
    ) -> Result<Self, CliError> {
        let parent_ref = SessionRef::of(parent_machine, parent);
        let (root, depth) = match &parent.lineage {
            Some(lineage) => (lineage.root.without_incarnation(), lineage.depth + 1),
            None => (parent_ref.without_incarnation(), 1),
        };
        if depth > MAX_DEPTH {
            return Err(CliError::usage(
                LINEAGE_DEPTH_EXCEEDED,
                format!("a session lineage is at most {MAX_DEPTH} deep"),
                Some(json!({ "parent": parent.id, "depth": depth })),
            ));
        }
        Ok(Self {
            machine: machine.to_string(),
            forge_context: crate::forge_identity::context_of(parent)?.map(|context| {
                LaunchContext {
                    role: None,
                    ..context
                }
            }),
            parent: Some(ParentLink {
                parent: parent_ref,
                root,
                depth,
            }),
            starter: Starter {
                kind: kind.to_string(),
                via: via.to_string(),
            },
        })
    }

    /// The stored lineage of the new record `id` created at `created_at`.
    pub(crate) fn finalize(&self, id: &str, created_at: &str) -> SessionLineage {
        let (parent, root, depth) = match &self.parent {
            Some(link) => (Some(link.parent.clone()), link.root.clone(), link.depth),
            None => (
                None,
                SessionRef {
                    machine: self.machine.clone(),
                    session_id: id.to_string(),
                    session_created_at: created_at.to_string(),
                    session_incarnation: None,
                },
                0,
            ),
        };
        SessionLineage {
            schema_version: LINEAGE_SCHEMA.to_string(),
            machine: self.machine.clone(),
            parent,
            root,
            depth,
            starter: self.starter.clone(),
            budget: None,
            extra: self
                .forge_context
                .as_ref()
                .map(|context| BTreeMap::from([(CONTEXT_KEY.into(), json!(context))]))
                .unwrap_or_default(),
        }
    }

    pub(crate) fn set_forge_context(&mut self, context: Option<LaunchContext>) {
        self.forge_context = context;
    }

    pub(crate) fn set_forge_role(&mut self, role: Option<String>) {
        if let Some(context) = self.forge_context.as_mut() {
            context.role = role;
        }
    }

    /// The `lineage` a console start sends to the target daemon. A root has
    /// no `root` yet: the target daemon names the session itself.
    pub(crate) fn to_create_json(&self) -> Value {
        let (parent, root, depth) = match &self.parent {
            Some(link) => (json!(link.parent), json!(link.root), link.depth),
            None => (Value::Null, Value::Null, 0),
        };
        let mut value = json!({
            "schema_version": LINEAGE_SCHEMA,
            "parent": parent,
            "root": root,
            "depth": depth,
            "starter": self.starter,
        });
        if let Some(context) = &self.forge_context {
            value[CONTEXT_KEY] = json!(context);
        }
        value
    }

    /// Validate the `lineage` of a create body for a session on `machine`.
    pub(crate) fn from_create_json(machine: &str, value: &Value) -> Result<Self, CliError> {
        if value.get(CONTEXT_KEY).is_some() {
            return Err(CliError::usage(
                "identity_session_binding_untrusted",
                "forge launch context requires a verified launch owner",
                None,
            ));
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RefInput {
            machine: String,
            session_id: String,
            session_created_at: String,
            #[serde(default)]
            session_incarnation: Option<String>,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct StarterInput {
            kind: String,
            via: String,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Input {
            #[serde(default)]
            schema_version: Option<String>,
            #[serde(default)]
            parent: Option<RefInput>,
            #[serde(default)]
            root: Option<RefInput>,
            depth: u32,
            starter: StarterInput,
        }
        fn checked(input: RefInput, keep_incarnation: bool) -> Result<SessionRef, CliError> {
            checked_ref(SessionRef {
                machine: input.machine,
                session_id: input.session_id,
                session_created_at: input.session_created_at,
                session_incarnation: input.session_incarnation.filter(|_| keep_incarnation),
            })
        }
        let input: Input = serde_json::from_value(value.clone()).map_err(|_| {
            lineage_invalid("lineage must be {schema_version, parent, root, depth, starter}")
        })?;
        if input
            .schema_version
            .as_deref()
            .is_some_and(|version| version != LINEAGE_SCHEMA)
        {
            return Err(lineage_invalid("unsupported lineage schema_version"));
        }
        let parented = matches!(
            input.starter.kind.as_str(),
            STARTER_SESSION | STARTER_MAIN_AGENT
        );
        let known_kind = parented
            || matches!(
                input.starter.kind.as_str(),
                STARTER_CONSOLE | STARTER_OPERATOR
            );
        let known_via = matches!(input.starter.via.as_str(), VIA_CLI | VIA_CONSOLE | VIA_HTTP);
        if !known_kind || !known_via {
            return Err(lineage_invalid("unknown lineage starter kind or via"));
        }
        let starter = Starter {
            kind: input.starter.kind,
            via: input.starter.via,
        };
        let parent = match (input.parent, input.root) {
            (None, None) if input.depth == 0 && !parented => None,
            (Some(parent), Some(root)) if input.depth >= 1 && parented => {
                if input.depth > MAX_DEPTH {
                    return Err(CliError::usage(
                        LINEAGE_DEPTH_EXCEEDED,
                        format!("a session lineage is at most {MAX_DEPTH} deep"),
                        Some(json!({ "depth": input.depth })),
                    ));
                }
                Some(ParentLink {
                    parent: checked(parent, true)?,
                    root: checked(root, false)?,
                    depth: input.depth,
                })
            }
            _ => {
                return Err(lineage_invalid(
                    "a session or main-agent start names its parent and root with depth 1 or more; a console or operator start names neither, with depth 0",
                ));
            }
        };
        Ok(Self {
            machine: machine.to_string(),
            parent,
            starter,
            forge_context: None,
        })
    }
}

/// Lineage and work for a plain CLI start or run. Inside a managed session
/// (`AGENT_SESSION_ID`), the caller is the parent when it resolves in this
/// state directory and `AGENT_SESSION_RUNTIME_ID` names its current runtime;
/// otherwise the new session is an operator root and the warning says why.
pub(crate) fn resolve_cli_start(
    context: &CliContext,
    no_parent: bool,
    work: &WorkRequest,
) -> Result<(crate::InitialLineage, Option<String>), CliError> {
    let machine = crate::board::machine_identity(None, context);
    let root = || crate::InitialLineage {
        seed: LineageSeed::root(&machine, STARTER_OPERATOR, VIA_CLI),
        work: work.resolve(None),
        role: None,
    };
    let caller = crate::non_empty_env("AGENT_SESSION_ID");
    if no_parent
        && let Some(caller) = &caller
        && let Ok(parent) = crate::load_session_record(context, caller)
        && let Some(launch) = crate::forge_identity::context_of(&parent)?
    {
        crate::forge_identity::authenticate_parent(context, &parent)?;
        let mut initial = root();
        initial.seed.set_forge_context(Some(LaunchContext {
            role: None,
            ..launch
        }));
        return Ok((initial, None));
    }
    let Some(caller) = caller.filter(|_| !no_parent) else {
        return Ok((root(), None));
    };
    let unresolved = |reason: &str| {
        Ok((
            root(),
            Some(format!(
                "AGENT_SESSION_ID {reason}; starting a new root session without a parent"
            )),
        ))
    };
    let Ok(parent) = crate::load_session_record(context, &caller) else {
        return unresolved("does not resolve in this state directory");
    };
    if crate::forge_identity::context_of(&parent)?.is_some() {
        crate::forge_identity::authenticate_parent(context, &parent)?;
    }
    let runtime = crate::non_empty_env("AGENT_SESSION_RUNTIME_ID");
    if runtime.is_none() || runtime.as_deref() != launch_id(&parent) {
        return unresolved("does not match AGENT_SESSION_RUNTIME_ID");
    }
    let machine = own_machine(context, &parent);
    let seed = LineageSeed::child_of(&machine, &machine, &parent, STARTER_SESSION, VIA_CLI)?;
    let work = work.resolve(parent.work.as_ref());
    Ok((
        crate::InitialLineage {
            seed,
            work,
            role: None,
        },
        None,
    ))
}

/// The label `record` was created under, so a child on the same machine names
/// its parent, and itself, the way the parent's creator did; this process's
/// own label for a record without lineage.
fn own_machine(context: &CliContext, record: &SessionRecord) -> String {
    record
        .lineage
        .as_ref()
        .map(|lineage| lineage.machine.clone())
        .filter(|machine| !machine.is_empty())
        .unwrap_or_else(|| crate::board::machine_identity(None, context))
}

/// Lineage and work for a Main Agent worker started by `owner`.
pub fn main_agent_worker(
    context: &CliContext,
    owner: &SessionRecord,
) -> Result<(LineageSeed, Option<SessionWork>), CliError> {
    let machine = own_machine(context, owner);
    let seed = LineageSeed::child_of(&machine, &machine, owner, STARTER_MAIN_AGENT, VIA_CLI)?;
    let work = WorkRequest {
        inherit: true,
        ..WorkRequest::default()
    }
    .resolve(owner.work.as_ref());
    Ok((seed, work))
}

/// A session reference with a bounded machine label, a valid session id, an
/// RFC 3339 creation time, and an optional bounded incarnation.
fn checked_ref(reference: SessionRef) -> Result<SessionRef, CliError> {
    let machine_ok = !reference.machine.is_empty()
        && reference.machine.len() <= MAX_MACHINE_BYTES
        && reference.machine.chars().all(|ch| ch.is_ascii_graphic());
    let time_ok = reference.session_created_at.len() <= MAX_TIMESTAMP_BYTES
        && reference
            .session_created_at
            .parse::<jiff::Timestamp>()
            .is_ok();
    let incarnation_ok = reference
        .session_incarnation
        .as_deref()
        .is_none_or(|value| {
            !value.is_empty()
                && value.len() <= MAX_INCARNATION_BYTES
                && value.chars().all(|ch| ch.is_ascii_graphic())
        });
    if !machine_ok
        || !time_ok
        || !incarnation_ok
        || crate::validate_id(&reference.session_id).is_err()
    {
        return Err(lineage_invalid("a lineage session reference is invalid"));
    }
    Ok(reference)
}

fn launch_id(record: &SessionRecord) -> Option<&str> {
    record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.trim())
        .filter(|launch_id| !launch_id.is_empty())
}

/// A later steward of a session (`lineage adopt`). `lineage` stays the
/// historical fact; readers use `adopted_by` when it is set.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct LineageAdoption {
    pub adopted_by: Option<SessionRef>,
    pub revision: u64,
    pub updated_at: String,
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// The session's effective parent: its steward when adopted, otherwise the
/// parent it was started by.
pub(crate) fn effective_parent(record: &SessionRecord) -> Option<&SessionRef> {
    match &record.lineage_adoption {
        Some(adoption) if adoption.adopted_by.is_some() => adoption.adopted_by.as_ref(),
        _ => record
            .lineage
            .as_ref()
            .and_then(|lineage| lineage.parent.as_ref()),
    }
}

/// The children check of a delete or archive: which scope was searched and
/// which children were orphaned by an explicit `--orphan-children`.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct ChildrenCheck {
    pub scope: &'static str,
    pub orphaned: Vec<SessionRef>,
}

/// Records in this state directory whose effective parent is `record`. A
/// record that cannot be read is skipped. Machine labels are not compared: a
/// child names its parent with the label of whichever process started it, and
/// `(session_id, session_created_at)` already identifies the parent.
pub(crate) fn local_children(
    context: &CliContext,
    machine: &str,
    record: &SessionRecord,
) -> Vec<SessionRef> {
    let Ok(entries) = std::fs::read_dir(context.state_dir.join("sessions")) else {
        return Vec::new();
    };
    let mut children: Vec<SessionRef> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
        .filter(|id| *id != record.id)
        .filter_map(|id| crate::load_session_record(context, &id).ok())
        .filter(|child| {
            effective_parent(child).is_some_and(|parent| {
                parent.session_id == record.id && parent.session_created_at == record.created_at
            })
        })
        .map(|child| SessionRef::of(machine, &child).without_incarnation())
        .collect();
    children.sort_by(|left, right| left.session_id.cmp(&right.session_id));
    children
}

/// Refuse to close `id` while sessions on this machine name it as their
/// effective parent, unless the caller acknowledged orphaning them. Child
/// references name this machine as `machine`.
pub(crate) fn guard_children(
    context: &CliContext,
    machine: &str,
    id: &str,
    orphan_children: bool,
) -> Result<ChildrenCheck, CliError> {
    let record = crate::load_session_record(context, id)?;
    let children = local_children(context, machine, &record);
    if !children.is_empty() && !orphan_children {
        return Err(CliError::data(
            SESSION_HAS_LIVE_CHILDREN,
            "the session still has children; close them first, or pass --orphan-children",
            Some(json!({ "children": children, "scope": "local" })),
        ));
    }
    Ok(ChildrenCheck {
        scope: "local",
        orphaned: children,
    })
}

/// Who runs a lineage or work mutation: the managed session this command
/// runs in, authenticated by its capability, or an operator outside any
/// managed session.
enum Caller {
    Session { id: String, created_at: String },
    Operator,
}

fn caller(context: &CliContext, capability_file: Option<&Path>) -> Result<Caller, CliError> {
    let Some(id) = crate::non_empty_env("AGENT_SESSION_ID") else {
        return Ok(Caller::Operator);
    };
    let token = crate::coordination::capability_token_from_file(capability_file)?;
    let (record, _) = crate::coordination::authenticate_token(context, &id, &token)?;
    Ok(Caller::Session {
        id: record.id,
        created_at: record.created_at,
    })
}

fn forbidden(code: &str, message: &str) -> CliError {
    CliError::data(code, message, None)
}

fn revision_conflict(code: &str, current: u64, expected: u64) -> CliError {
    CliError::data(
        code,
        "the revision changed; read it again and retry",
        Some(json!({ "current_revision": current, "expected_revision": expected })),
    )
}

pub(crate) fn run_lineage(context: &CliContext, args: crate::cli::LineageArgs) -> i32 {
    match args.command {
        crate::cli::LineageCommand::Adopt(args) => {
            let format = args.format;
            match adopt(context, args) {
                Ok(result) => crate::render_single_success(
                    LINEAGE_ADOPT_COMMAND,
                    format,
                    &result,
                    render_adopt_text,
                ),
                Err(error) => crate::render_error(LINEAGE_ADOPT_COMMAND, format, error),
            }
        }
    }
}

pub(crate) fn run_work(context: &CliContext, args: crate::cli::WorkArgs) -> i32 {
    match args.command {
        crate::cli::WorkCommand::Set(args) => {
            let format = args.format;
            match set_work(context, args) {
                Ok(result) => crate::render_single_success(
                    WORK_SET_COMMAND,
                    format,
                    &result,
                    render_work_text,
                ),
                Err(error) => crate::render_error(WORK_SET_COMMAND, format, error),
            }
        }
    }
}

fn adopt(context: &CliContext, args: crate::cli::LineageAdoptArgs) -> Result<Value, CliError> {
    crate::validate_id(&args.child)?;
    let caller = caller(context, args.capability_file.as_deref())?;
    let steward = match &args.by {
        None => None,
        Some(by) => Some(steward_ref(context, by, &args)?),
    };
    if let Caller::Session { id, created_at } = &caller {
        // A remote steward cannot be the caller: a managed session names
        // itself only through its own local record.
        let own = args.by_machine.is_none()
            && steward.as_ref().is_some_and(|steward| {
                steward.session_id == *id && steward.session_created_at == *created_at
            });
        if !own {
            return Err(forbidden(
                LINEAGE_ADOPT_FORBIDDEN,
                "a managed session may only adopt children for itself; run as an operator to name another steward or clear one",
            ));
        }
    }
    crate::mutate_session_record(context, &args.child, |record| {
        if steward
            .as_ref()
            .is_some_and(|steward| reaches(context, steward, &record.id, &record.created_at))
        {
            return Err(CliError::usage(
                LINEAGE_INVALID,
                "a session cannot adopt itself or one of its own stewards or ancestors",
                None,
            ));
        }
        let current = record
            .lineage_adoption
            .as_ref()
            .map_or(0, |adoption| adoption.revision);
        if let Some(expected) = args.if_revision
            && expected != current
        {
            return Err(revision_conflict(
                LINEAGE_REVISION_CONFLICT,
                current,
                expected,
            ));
        }
        let now = jiff::Timestamp::now().to_string();
        record.lineage_adoption = Some(LineageAdoption {
            adopted_by: steward.clone(),
            revision: current + 1,
            updated_at: now.clone(),
            extra: BTreeMap::new(),
        });
        record.updated_at = now;
        Ok(json!({
            "session_id": record.id,
            "lineage": record.lineage,
            "lineage_adoption": record.lineage_adoption,
            "effective_parent": effective_parent(record),
        }))
    })
}

/// Whether following effective parents up from `start` through this state
/// directory reaches the session `(id, created_at)`, which would make an
/// adoption loop. A reference that is not local ends the walk.
fn reaches(context: &CliContext, start: &SessionRef, id: &str, created_at: &str) -> bool {
    let mut current = Some(start.clone());
    for _ in 0..=MAX_DEPTH {
        let Some(reference) = current.take() else {
            return false;
        };
        if reference.session_id == id && reference.session_created_at == created_at {
            return true;
        }
        let Ok(record) = crate::load_session_record(context, &reference.session_id) else {
            return false;
        };
        if record.created_at != reference.session_created_at {
            return false;
        }
        current = effective_parent(&record).cloned();
    }
    false
}

/// `--by` as a session reference: a local session resolves to its exact
/// identity and current incarnation; a steward on another machine is named
/// with both `--by-machine` and `--by-created-at` (clap requires the pair).
fn steward_ref(
    context: &CliContext,
    by: &str,
    args: &crate::cli::LineageAdoptArgs,
) -> Result<SessionRef, CliError> {
    crate::validate_id(by)?;
    if let (Some(by_machine), Some(created_at)) = (&args.by_machine, &args.by_created_at) {
        return checked_ref(SessionRef {
            machine: by_machine.clone(),
            session_id: by.to_string(),
            session_created_at: created_at.clone(),
            session_incarnation: None,
        });
    }
    let record = crate::load_session_record(context, by)?;
    Ok(SessionRef::of(&own_machine(context, &record), &record))
}

fn set_work(context: &CliContext, args: crate::cli::WorkSetArgs) -> Result<Value, CliError> {
    crate::validate_id(&args.id)?;
    let program = args.program.as_deref().map(WorkRef::parse).transpose()?;
    let issues = if args.issues.is_empty() {
        None
    } else {
        Some(canonical_issues(
            args.issues
                .iter()
                .map(|issue| WorkRef::parse(issue))
                .collect::<Result<Vec<_>, _>>()?,
        )?)
    };
    if program.is_none() && issues.is_none() && !args.clear_program && !args.clear_issues {
        return Err(CliError::usage(
            WORK_REF_INVALID,
            "work set needs --program, --issue, --clear-program, or --clear-issues",
            None,
        ));
    }
    if let Caller::Session { id, .. } = caller(context, args.capability_file.as_deref())?
        && id != args.id
    {
        return Err(forbidden(
            WORK_SET_FORBIDDEN,
            "a managed session may only set its own work; run as an operator to set another session's",
        ));
    }
    crate::mutate_session_record(context, &args.id, |record| {
        let current = record.work.as_ref().map_or(0, |work| work.revision);
        if args.if_revision != current {
            return Err(revision_conflict(
                WORK_REVISION_CONFLICT,
                current,
                args.if_revision,
            ));
        }
        let mut work = record.work.clone().unwrap_or(SessionWork {
            program: None,
            issues: Vec::new(),
            inherited: false,
            revision: 0,
            extra: BTreeMap::new(),
        });
        if args.clear_program {
            work.program = None;
        }
        if args.clear_issues {
            work.issues = Vec::new();
        }
        if let Some(program) = program {
            work.program = Some(program);
        }
        if let Some(issues) = issues {
            work.issues = issues;
        }
        work.inherited = false;
        work.revision = current + 1;
        record.work = Some(work);
        record.updated_at = jiff::Timestamp::now().to_string();
        Ok(json!({ "session_id": record.id, "work": record.work }))
    })
}

fn render_adopt_text(result: &Value) -> String {
    let id = result["session_id"].as_str().unwrap_or_default();
    match result["lineage_adoption"]["adopted_by"]["session_id"].as_str() {
        Some(steward) => format!("{id} is now adopted by {steward}\n"),
        None => format!("{id} has no steward; its parent is its effective parent\n"),
    }
}

fn render_work_text(result: &Value) -> String {
    let work = &result["work"];
    let name = |reference: &Value| {
        let provider = reference["provider"].as_str().unwrap_or_default();
        let prefix = if provider == "github" {
            String::new()
        } else {
            format!("{provider}:")
        };
        format!(
            "{prefix}{}#{}",
            reference["repository"].as_str().unwrap_or_default(),
            reference["number"]
        )
    };
    let program = if work["program"].is_null() {
        "none".to_string()
    } else {
        name(&work["program"])
    };
    let issues = work["issues"]
        .as_array()
        .map(|issues| issues.iter().map(name).collect::<Vec<_>>().join(", "))
        .filter(|issues| !issues.is_empty())
        .unwrap_or_else(|| "none".to_string());
    format!(
        "work for {} (revision {}): program {program}; issues {issues}\n",
        result["session_id"].as_str().unwrap_or_default(),
        work["revision"]
    )
}

fn lineage_invalid(message: &str) -> CliError {
    CliError::usage(LINEAGE_INVALID, message, None)
}

fn work_invalid(message: &str) -> CliError {
    CliError::usage(WORK_REF_INVALID, message, None)
}

fn bounded(value: &str) -> String {
    value.chars().take(128).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn issue(repository: &str, number: u64) -> WorkRef {
        WorkRef {
            provider: "github".to_string(),
            repository: repository.to_string(),
            number,
        }
    }

    fn work(program: Option<WorkRef>, issues: Vec<WorkRef>) -> SessionWork {
        SessionWork {
            program,
            issues,
            inherited: false,
            revision: 3,
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn work_refs_parse_only_the_public_grammar() {
        assert_eq!(
            WorkRef::parse("Sympoies/Nils-CLI#2032").unwrap(),
            issue("sympoies/nils-cli", 2032)
        );
        assert_eq!(
            WorkRef::parse("gitlab:group/repo#7").unwrap(),
            WorkRef {
                provider: "gitlab".to_string(),
                repository: "group/repo".to_string(),
                number: 7,
            }
        );
        for value in [
            "",
            "fix the thing",
            "owner/repo",
            "owner/repo#",
            "owner/repo#0",
            "owner/repo#-1",
            "owner/repo#+1",
            "owner/repo#1x",
            "owner#1",
            "a/b/c#1",
            "bitbucket:owner/repo#1",
            "owner/repo #1",
        ] {
            let error = WorkRef::parse(value).expect_err(value).into_inner();
            assert_eq!(error.code, WORK_REF_INVALID, "{value}");
        }
    }

    #[test]
    fn work_inherits_each_dimension_unless_given_or_opted_out() {
        let parent = work(
            Some(issue("serenvia/laoda", 44)),
            vec![issue("sympoies/nils-cli", 2032)],
        );
        let inherit = WorkRequest::from_flags(None, &[], false).unwrap();
        let resolved = inherit.resolve(Some(&parent)).unwrap();
        assert_eq!(
            (
                resolved.program.clone(),
                resolved.issues.clone(),
                resolved.inherited,
                resolved.revision
            ),
            (parent.program.clone(), parent.issues.clone(), true, 1)
        );

        let issues_only =
            WorkRequest::from_flags(None, &["sympoies/nils-cli#9".to_string()], false).unwrap();
        let resolved = issues_only.resolve(Some(&parent)).unwrap();
        assert_eq!(
            (resolved.program, resolved.issues, resolved.inherited),
            (
                parent.program.clone(),
                vec![issue("sympoies/nils-cli", 9)],
                false
            )
        );

        let opted_out = WorkRequest::from_flags(None, &[], true).unwrap();
        assert_eq!(opted_out.resolve(Some(&parent)), None);
        assert_eq!(inherit.resolve(None), None);

        let program_only = WorkRequest::from_flags(Some("serenvia/laoda#45"), &[], true).unwrap();
        let resolved = program_only.resolve(Some(&parent)).unwrap();
        assert_eq!(
            (resolved.program, resolved.issues, resolved.inherited),
            (Some(issue("serenvia/laoda", 45)), Vec::new(), false)
        );
    }

    #[test]
    fn work_names_at_most_four_distinct_issues() {
        let four = ["a/b#1", "a/b#2", "a/b#3", "a/b#4", "A/B#4"].map(str::to_string);
        let request = WorkRequest::from_flags(None, &four, false).unwrap();
        assert_eq!(request.issues.unwrap().len(), 4);
        let five = ["a/b#1", "a/b#2", "a/b#3", "a/b#4", "a/b#5"].map(str::to_string);
        let error = WorkRequest::from_flags(None, &five, false)
            .unwrap_err()
            .into_inner();
        assert_eq!(error.code, WORK_REF_INVALID);
    }

    #[test]
    fn console_work_requests_round_trip() {
        let request = WorkRequest::from_flags(Some("a/b#1"), &["c/d#2".to_string()], true).unwrap();
        let value = request.to_request_json().unwrap();
        assert_eq!(WorkRequest::from_request_json(&value).unwrap(), request);
        let default = WorkRequest::from_flags(None, &[], false).unwrap();
        assert_eq!(default.to_request_json(), None);
        for value in [
            json!({"program": "a/b#1"}),
            json!({"issues": [{"provider": "github", "repository": "a/b", "number": 0}]}),
            json!({"inherit": true, "extra": 1}),
        ] {
            assert!(WorkRequest::from_request_json(&value).is_err(), "{value}");
        }
    }

    #[test]
    fn main_agent_workers_are_children_of_their_owner_and_inherit_its_work() {
        let context = CliContext {
            state_dir: std::path::PathBuf::from("/nonexistent"),
            host: Some("lineage-host".to_string()),
        };
        let machine = crate::board::machine_identity(None, &context);
        let owner: SessionRecord = serde_json::from_value(json!({
            "schema_version": "agent-session.session.v1",
            "id": "main-owner",
            "agent": "codex",
            "mode": "interactive",
            "title": null,
            "cwd": "/w",
            "tmux_session": "hs-codex-main-owner",
            "prompt_file": null,
            "log_file": null,
            "created_at": "2026-10-01T00:00:00Z",
            "updated_at": "2026-10-01T00:00:00Z",
            "runtime": {
                "kind": "tmux",
                "tmux_session": "hs-codex-main-owner",
                "generation": 2,
                "started_at": "2026-10-01T00:00:00Z",
                "launch_id": "owner-launch"
            },
            "work": {"program": {"provider": "github", "repository": "a/b", "number": 1},
                     "issues": [], "inherited": false, "revision": 4}
        }))
        .unwrap();
        let (seed, work) = main_agent_worker(&context, &owner).unwrap();
        let lineage = seed.finalize("worker", "2026-10-01T01:00:00Z");
        let owner_ref = json!({
            "machine": machine,
            "session_id": "main-owner",
            "session_created_at": "2026-10-01T00:00:00Z",
        });
        let mut parent = owner_ref.clone();
        parent["session_incarnation"] = json!("owner-launch");
        assert_eq!(
            json!(lineage),
            json!({
                "schema_version": LINEAGE_SCHEMA,
                "machine": machine,
                "parent": parent,
                "root": owner_ref,
                "depth": 1,
                "starter": {"kind": "main-agent", "via": "cli"},
                "budget": null,
            })
        );
        let work = work.unwrap();
        assert_eq!(
            (work.program, work.issues, work.inherited, work.revision),
            (Some(issue("a/b", 1)), Vec::new(), true, 1)
        );
    }

    #[test]
    fn a_child_may_be_64_deep_but_not_deeper() {
        let mut parent: SessionRecord = serde_json::from_value(json!({
            "schema_version": "agent-session.session.v1",
            "id": "deep-parent",
            "agent": "codex",
            "mode": "interactive",
            "title": null,
            "cwd": "/w",
            "tmux_session": "hs-codex-deep-parent",
            "prompt_file": null,
            "log_file": null,
            "created_at": "2026-10-01T00:00:00Z",
            "updated_at": "2026-10-01T00:00:00Z"
        }))
        .unwrap();
        let root = SessionRef {
            machine: "h".to_string(),
            session_id: "root".to_string(),
            session_created_at: "2026-10-01T00:00:00Z".to_string(),
            session_incarnation: None,
        };
        let mut lineage = LineageSeed::root("h", STARTER_OPERATOR, VIA_CLI)
            .finalize("deep-parent", "2026-10-01T00:00:00Z");
        lineage.root = root;
        lineage.depth = MAX_DEPTH - 1;
        parent.lineage = Some(lineage.clone());
        let seed = LineageSeed::child_of("h", "h", &parent, STARTER_SESSION, VIA_CLI).unwrap();
        assert_eq!(
            seed.finalize("child", "2026-10-01T01:00:00Z").depth,
            MAX_DEPTH
        );
        lineage.depth = MAX_DEPTH;
        parent.lineage = Some(lineage);
        let error = LineageSeed::child_of("h", "h", &parent, STARTER_SESSION, VIA_CLI)
            .unwrap_err()
            .into_inner();
        assert_eq!(error.code, LINEAGE_DEPTH_EXCEEDED);
    }

    #[test]
    fn create_body_lineage_round_trips_and_rejects_inconsistent_shapes() {
        let parent = SessionRef {
            machine: "sympoies".to_string(),
            session_id: "85a7379c-0bfc-4675-bab9-31ed8dcfce92".to_string(),
            session_created_at: "2026-10-01T00:00:00Z".to_string(),
            session_incarnation: Some("launch-1".to_string()),
        };
        let seed = LineageSeed {
            forge_context: None,
            machine: "c8".to_string(),
            parent: Some(ParentLink {
                parent: parent.clone(),
                root: parent.without_incarnation(),
                depth: 1,
            }),
            starter: Starter {
                kind: STARTER_SESSION.to_string(),
                via: VIA_CONSOLE.to_string(),
            },
        };
        let value = seed.to_create_json();
        assert_eq!(LineageSeed::from_create_json("c8", &value).unwrap(), seed);

        let root = LineageSeed::root("c8", STARTER_CONSOLE, VIA_CONSOLE);
        let lineage = LineageSeed::from_create_json("c8", &root.to_create_json())
            .unwrap()
            .finalize("child", "2026-10-01T01:00:00Z");
        assert_eq!(
            json!(lineage),
            json!({
                "schema_version": LINEAGE_SCHEMA,
                "machine": "c8",
                "parent": null,
                "root": {"machine": "c8", "session_id": "child", "session_created_at": "2026-10-01T01:00:00Z"},
                "depth": 0,
                "starter": {"kind": "console", "via": "console"},
                "budget": null,
            })
        );

        let mut cases = Vec::new();
        let mut no_root = value.clone();
        no_root["root"] = Value::Null;
        cases.push(no_root);
        let mut depth_zero = value.clone();
        depth_zero["depth"] = json!(0);
        cases.push(depth_zero);
        let mut operator_with_parent = value.clone();
        operator_with_parent["starter"]["kind"] = json!("operator");
        cases.push(operator_with_parent);
        let mut unknown_kind = value.clone();
        unknown_kind["starter"]["kind"] = json!("robot");
        cases.push(unknown_kind);
        let mut bad_id = value.clone();
        bad_id["parent"]["session_id"] = json!("../escape");
        cases.push(bad_id);
        let mut bad_time = value.clone();
        bad_time["parent"]["session_created_at"] = json!("yesterday");
        cases.push(bad_time);
        let mut extra = value.clone();
        extra["owner"] = json!("someone");
        cases.push(extra);
        let mut schema = value.clone();
        schema["schema_version"] = json!("agent-session.session-lineage.v2");
        cases.push(schema);
        let mut session_root = root.to_create_json();
        session_root["starter"]["kind"] = json!("session");
        cases.push(session_root);
        for case in cases {
            let error = LineageSeed::from_create_json("c8", &case)
                .expect_err(&case.to_string())
                .into_inner();
            assert_eq!(error.code, LINEAGE_INVALID, "{case}");
        }
        let mut deep = value.clone();
        deep["depth"] = json!(MAX_DEPTH + 1);
        let error = LineageSeed::from_create_json("c8", &deep)
            .unwrap_err()
            .into_inner();
        assert_eq!(error.code, LINEAGE_DEPTH_EXCEEDED);
    }
}
