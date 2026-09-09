//! WorkspaceLease v2: target-scoped repository authority.
//!
//! v1 bound one immutable session cwd, so a dirty anchor denied every tool. v2
//! classifies each exact operation into zero or more canonical repository
//! targets and binds them lazily and independently.

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use pretty_assertions::{assert_eq, assert_ne};
use serde_json::{Value, json};

use support::Fixture;

const POLICY: &str = r#"schema_version = "agent-hook.policy.v1"
bundle_id = "workspace-lease-v2-test"
version = "2026.09.02.1"
"#;

const RESOLVE_RESULT: &str = "agent-hook.workspace-lease.resolve-result.v2";
const BIND_RESULT: &str = "agent-hook.workspace-lease.bind-result.v2";
const BEGIN_RESULT: &str = "agent-hook.workspace-lease.begin-result.v2";
const COMPLETE_RESULT: &str = "agent-hook.workspace-lease.complete-result.v2";
const RENEW_RESULT: &str = "agent-hook.workspace-lease.renew-result.v2";
const RELEASE_RESULT: &str = "agent-hook.workspace-lease.release-result.v2";

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("git command");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn repo(root: &Path) -> PathBuf {
    fs::create_dir_all(root).expect("repository directory");
    git(root, &["init", "--quiet"]);
    git(root, &["config", "user.email", "workspace@example.com"]);
    git(root, &["config", "user.name", "Workspace Test"]);
    fs::write(root.join("tracked.txt"), "base\n").expect("tracked file");
    git(root, &["add", "--all"]);
    git(root, &["commit", "--quiet", "-m", "test: initial"]);
    fs::canonicalize(root).expect("canonical repository root")
}

fn invoke(fixture: &Fixture, operation: &str, request: Value) -> (i32, Value) {
    let output = fixture.run(
        &["workspace-lease", operation, "--format", "json"],
        Some(&request.to_string()),
    );
    (output.code, output.stdout_json())
}

fn ok(fixture: &Fixture, operation: &str, request: Value) -> Value {
    let (code, envelope) = invoke(fixture, operation, request);
    assert_eq!(code, 0, "envelope={envelope}");
    envelope["data"].clone()
}

fn resolve_request(
    session: &str,
    request_id: &str,
    anchor: Option<&Path>,
    tool: &str,
    arguments: Value,
) -> Value {
    let mut request = json!({
        "schema_version": "agent-hook.workspace-lease.resolve.v2",
        "version": 2,
        "request_id": request_id,
        "session_id": session,
        "call_id": format!("call:{request_id}"),
        "root_call_id": format!("root:{request_id}"),
        "tool_name": tool,
        "arguments": arguments,
        "nested": false
    });
    if let Some(anchor) = anchor {
        request["anchor_cwd"] = json!(anchor);
    }
    request
}

fn resolve(
    fixture: &Fixture,
    session: &str,
    request_id: &str,
    anchor: Option<&Path>,
    tool: &str,
    arguments: Value,
) -> Value {
    ok(
        fixture,
        "resolve",
        resolve_request(session, request_id, anchor, tool, arguments),
    )
}

fn write_targets(fixture: &Fixture, session: &str, request_id: &str, path: &Path) -> Value {
    let data = resolve(
        fixture,
        session,
        request_id,
        None,
        "write",
        json!({"file_path": path, "content": "next"}),
    );
    assert_eq!(data["schema_version"], RESOLVE_RESULT);
    data
}

fn bind_request(session: &str, request_id: &str, target: &Value) -> Value {
    json!({
        "schema_version": "agent-hook.workspace-lease.bind.v2",
        "version": 2,
        "request_id": request_id,
        "session_id": session,
        "target": target,
        "source": "startup"
    })
}

fn bind(fixture: &Fixture, session: &str, request_id: &str, target: &Value) -> Value {
    ok(fixture, "bind", bind_request(session, request_id, target))
}

fn anchor_bind_request(session: &str, request_id: &str, cwd: &Path, source: &str) -> Value {
    json!({
        "schema_version": "agent-hook.workspace-lease.bind.v2",
        "version": 2,
        "request_id": request_id,
        "session_id": session,
        "cwd": cwd,
        "source": source
    })
}

/// The execution a `begin` declares: the call `resolve` classified, plus the
/// token it minted for the target being fenced.
struct BeginCall<'a> {
    tool: &'a str,
    arguments: Value,
    anchor: Option<&'a Path>,
    token: &'a Value,
}

fn begin_request(
    session: &str,
    request_id: &str,
    binding: &Value,
    target: &Value,
    call: BeginCall<'_>,
) -> Value {
    json!({
        "schema_version": "agent-hook.workspace-lease.begin.v2",
        "version": 2,
        "request_id": request_id,
        "session_id": session,
        "binding_id": binding["binding_id"],
        "workspace_id": binding["workspace_id"],
        "generation": binding["generation"],
        "binding_state": binding["state"],
        "call_id": format!("call:{request_id}"),
        "root_call_id": format!("root:{request_id}"),
        "tool_name": call.tool,
        "arguments": call.arguments,
        "nested": false,
        "anchor_cwd": call.anchor,
        "target": target,
        "target_token": call.token
    })
}

fn begin(
    fixture: &Fixture,
    session: &str,
    request_id: &str,
    binding: &Value,
    target: &Value,
) -> Value {
    let arguments = write_arguments(target);
    let token = token_for(
        fixture,
        session,
        request_id,
        "write",
        arguments.clone(),
        None,
        &target["workspace_key"],
    );
    ok(
        fixture,
        "begin",
        begin_request(
            session,
            request_id,
            binding,
            target,
            BeginCall {
                tool: "write",
                arguments,
                anchor: None,
                token: &token,
            },
        ),
    )
}

/// A write whose declared path proves the target repository, so `resolve`
/// classifies the same workspace the `begin` names.
fn write_arguments(target: &Value) -> Value {
    let root = Path::new(target["root"].as_str().expect("target root"));
    json!({"file_path": root.join("tracked.txt"), "content": "next"})
}

fn complete(
    fixture: &Fixture,
    session: &str,
    request_id: &str,
    binding: &Value,
    operation: &Value,
    begin_request_id: &str,
) -> Value {
    let data = ok(
        fixture,
        "complete",
        json!({
            "schema_version": "agent-hook.workspace-lease.complete.v2",
            "version": 2,
            "request_id": request_id,
            "session_id": session,
            "binding_id": binding["binding_id"],
            "workspace_id": binding["workspace_id"],
            "generation": binding["generation"],
            "operation_id": operation["operation_id"],
            "fence": operation["fence"],
            "call_id": format!("call:{begin_request_id}"),
            "root_call_id": format!("root:{begin_request_id}"),
            "tool_name": "write",
            "outcome": "succeeded"
        }),
    );
    assert_eq!(data["schema_version"], COMPLETE_RESULT, "data={data}");
    data
}

fn release(fixture: &Fixture, session: &str, request_id: &str, binding: &Value) -> Value {
    let data = ok(
        fixture,
        "release",
        json!({
            "schema_version": "agent-hook.workspace-lease.release.v2",
            "version": 2,
            "request_id": request_id,
            "session_id": session,
            "binding_id": binding["binding_id"],
            "workspace_id": binding["workspace_id"],
            "generation": binding["generation"],
            "reason": "agent-disposed"
        }),
    );
    assert_eq!(data["schema_version"], RELEASE_RESULT, "data={data}");
    data
}

fn renew_request(session: &str, request_id: &str, binding: &Value) -> Value {
    json!({
        "schema_version": "agent-hook.workspace-lease.renew.v2",
        "version": 2,
        "request_id": request_id,
        "session_id": session,
        "binding_id": binding["binding_id"],
        "workspace_id": binding["workspace_id"],
        "generation": binding["generation"]
    })
}

fn only_resolved(data: &Value) -> Value {
    assert_eq!(data["kind"], "targets", "data={data}");
    let targets = data["targets"].as_array().expect("target array");
    assert_eq!(targets.len(), 1, "data={data}");
    targets[0].clone()
}

/// The target reference `bind` and `begin` accept, without the resolve token.
fn only_target(data: &Value) -> Value {
    let resolved = only_resolved(data);
    json!({
        "workspace_key": resolved["workspace_key"],
        "root": resolved["root"],
    })
}

/// The token `resolve` minted for the sole target of this classification.
fn only_token(data: &Value) -> Value {
    let resolved = only_resolved(data);
    let token = resolved["token"].clone();
    assert!(
        token.as_str().is_some_and(|value| value.len() == 64),
        "resolved={resolved}"
    );
    token
}

/// Mint the resolve token for exactly the call facts a `begin` will present.
///
/// A correct runtime resolves and begins the same tool call, so the token it
/// passes back is keyed over these same facts. Reproducing that here is what
/// makes the fence testable without letting `begin` reclassify anything.
fn token_for(
    fixture: &Fixture,
    session: &str,
    request_id: &str,
    tool: &str,
    arguments: Value,
    anchor: Option<&Path>,
    workspace_key: &Value,
) -> Value {
    let data = resolve(fixture, session, request_id, anchor, tool, arguments);
    let targets = data["targets"].as_array().expect("target array");
    let matched = targets
        .iter()
        .find(|entry| &entry["workspace_key"] == workspace_key)
        .unwrap_or_else(|| panic!("classified target for {workspace_key}: data={data}"));
    matched["token"].clone()
}

#[test]
fn unclassifiable_and_read_only_operations_need_no_repository_target() {
    let fixture = Fixture::new(POLICY);
    let root = repo(&fixture.root.join("repo-a"));

    for (tool, arguments) in [
        (
            "bash",
            json!({"command": "rm -rf ./build", "workdir": root}),
        ),
        ("read", json!({"file_path": root.join("tracked.txt")})),
        (
            "str_replace_editor",
            json!({"command": "view", "path": root.join("tracked.txt")}),
        ),
        ("some_future_tool", json!({"file_path": root})),
    ] {
        let data = resolve(
            &fixture,
            "session-a",
            &format!("r-{tool}"),
            None,
            tool,
            arguments,
        );
        assert_eq!(data["schema_version"], RESOLVE_RESULT);
        assert_eq!(data["kind"], "not-required", "tool={tool} data={data}");
    }
}

#[test]
fn non_repository_writes_need_no_repository_target() {
    let fixture = Fixture::new(POLICY);
    let plain = fixture.root.join("plain/nested");
    fs::create_dir_all(&plain).expect("plain directory");

    let data = write_targets(&fixture, "session-a", "r-plain", &plain.join("notes.txt"));
    assert_eq!(data["kind"], "not-required", "data={data}");
}

#[test]
fn path_spellings_converge_and_relative_paths_resolve_from_the_anchor() {
    let fixture = Fixture::new(POLICY);
    let root = repo(&fixture.root.join("repo-a"));
    fs::create_dir_all(root.join("nested")).expect("nested");
    let link = fixture.root.join("link-a");
    std::os::unix::fs::symlink(&root, &link).expect("symlink");

    let direct = only_target(&write_targets(
        &fixture,
        "session-a",
        "r-direct",
        &root.join("tracked.txt"),
    ));
    let dotted = only_target(&write_targets(
        &fixture,
        "session-a",
        "r-dotted",
        &root.join("nested/../tracked.txt"),
    ));
    let linked = only_target(&write_targets(
        &fixture,
        "session-a",
        "r-linked",
        &link.join("tracked.txt"),
    ));
    let relative = only_target(&resolve(
        &fixture,
        "session-a",
        "r-relative",
        Some(root.join("nested").as_path()),
        "write",
        json!({"file_path": "../tracked.txt", "content": "next"}),
    ));

    assert_eq!(direct["root"], json!(root));
    for other in [&dotted, &linked, &relative] {
        assert_eq!(other, &direct, "targets must converge");
    }
}

#[test]
fn a_relative_target_without_an_anchor_fails_closed() {
    let fixture = Fixture::new(POLICY);
    repo(&fixture.root.join("repo-a"));

    let (code, envelope) = invoke(
        &fixture,
        "resolve",
        resolve_request(
            "session-a",
            "r-relative",
            None,
            "write",
            json!({"file_path": "tracked.txt", "content": "next"}),
        ),
    );
    assert_ne!(code, 0, "envelope={envelope}");
    assert_eq!(envelope["error"]["code"], "workspace-target-unresolvable");
}

#[test]
fn distinct_repositories_bind_independently_for_one_session() {
    let fixture = Fixture::new(POLICY);
    let first = repo(&fixture.root.join("repo-a"));
    let second = repo(&fixture.root.join("repo-b"));

    let target_a = only_target(&write_targets(
        &fixture,
        "session-a",
        "r-a",
        &first.join("tracked.txt"),
    ));
    let target_b = only_target(&write_targets(
        &fixture,
        "session-a",
        "r-b",
        &second.join("tracked.txt"),
    ));
    assert_ne!(target_a["workspace_key"], target_b["workspace_key"]);

    let binding_a = bind(&fixture, "session-a", "b-a", &target_a);
    let binding_b = bind(&fixture, "session-a", "b-b", &target_b);
    assert_eq!(binding_a["schema_version"], BIND_RESULT);
    assert_eq!(binding_a["kind"], "bound");
    assert_eq!(binding_a["state"], "owned");
    assert_eq!(binding_b["kind"], "bound");
    assert_ne!(binding_a["binding_id"], binding_b["binding_id"]);
    assert_ne!(binding_a["workspace_id"], binding_b["workspace_id"]);

    // A binding for A grants no authority over B and vice versa. Identity is
    // authenticated before the call binding, so this stays target-invalid even
    // with an honest token for B.
    let cross_arguments = write_arguments(&target_b);
    let cross_token = token_for(
        &fixture,
        "session-a",
        "x-cross",
        "write",
        cross_arguments.clone(),
        None,
        &target_b["workspace_key"],
    );
    let (code, envelope) = invoke(
        &fixture,
        "begin",
        begin_request(
            "session-a",
            "x-cross",
            &binding_a,
            &target_b,
            BeginCall {
                tool: "write",
                arguments: cross_arguments,
                anchor: None,
                token: &cross_token,
            },
        ),
    );
    assert_ne!(code, 0, "envelope={envelope}");
    assert_eq!(envelope["error"]["code"], "workspace-target-invalid");

    let operation_a = begin(&fixture, "session-a", "g-a", &binding_a, &target_a);
    let operation_b = begin(&fixture, "session-a", "g-b", &binding_b, &target_b);
    assert_eq!(operation_a["schema_version"], BEGIN_RESULT);
    assert_eq!(operation_a["kind"], "granted");
    assert_eq!(operation_b["kind"], "granted");
    assert_ne!(operation_a["fence"], operation_b["fence"]);

    assert_eq!(
        complete(
            &fixture,
            "session-a",
            "c-a",
            &binding_a,
            &operation_a,
            "g-a"
        )["kind"],
        "completed"
    );
    assert_eq!(
        complete(
            &fixture,
            "session-a",
            "c-b",
            &binding_b,
            &operation_b,
            "g-b"
        )["kind"],
        "completed"
    );
    assert_eq!(
        release(&fixture, "session-a", "rel-a", &binding_a)["kind"],
        "released"
    );
    assert_eq!(
        release(&fixture, "session-a", "rel-b", &binding_b)["kind"],
        "released"
    );
}

#[test]
fn repeated_resolution_of_one_target_set_is_stable() {
    let fixture = Fixture::new(POLICY);
    let root = repo(&fixture.root.join("repo-a"));

    // Repeated resolution of the same repository projects a byte-stable target
    // set. Every tool classified today yields at most one target, so the sorted
    // keyed-digest ordering this relies on is the wire contract for a future
    // multi-path tool rather than a rule exercised here.
    let first = write_targets(&fixture, "session-a", "r-set-1", &root.join("tracked.txt"));
    let second = write_targets(
        &fixture,
        "session-a",
        "r-set-2",
        &root.join("nested/new.txt"),
    );
    assert_eq!(only_target(&first), only_target(&second));
}

#[test]
fn a_dirty_target_denies_only_that_repository_while_a_clean_worktree_binds() {
    let fixture = Fixture::new(POLICY);
    let dirty_root = repo(&fixture.root.join("repo-a"));
    let clean_root = repo(&fixture.root.join("repo-b"));
    let linked = fixture.root.join("linked-a");
    git(
        &dirty_root,
        &[
            "worktree",
            "add",
            "--quiet",
            linked.to_str().expect("linked path"),
            "-b",
            "linked",
        ],
    );
    let linked = fs::canonicalize(&linked).expect("canonical linked worktree");
    fs::write(dirty_root.join("tracked.txt"), "dirty\n").expect("dirty file");

    let dirty_target = only_target(&write_targets(
        &fixture,
        "session-a",
        "r-dirty",
        &dirty_root.join("tracked.txt"),
    ));
    let denial = bind(&fixture, "session-a", "b-dirty", &dirty_target);
    assert_eq!(denial["schema_version"], BIND_RESULT);
    assert_eq!(denial["kind"], "denied");
    assert_eq!(denial["state"], "dirty");
    assert_eq!(denial["code"], "WORKSPACE_DIRTY");

    // The same session keeps full authority over an unrelated repository and
    // over a distinct clean linked worktree of the same repository.
    for (request_id, root) in [("b-clean", &clean_root), ("b-linked", &linked)] {
        let target = only_target(&write_targets(
            &fixture,
            "session-a",
            &format!("r-{request_id}"),
            &root.join("tracked.txt"),
        ));
        assert_ne!(target["workspace_key"], dirty_target["workspace_key"]);
        let binding = bind(&fixture, "session-a", request_id, &target);
        assert_eq!(binding["kind"], "bound", "root={root:?}");
        assert_eq!(
            begin(
                &fixture,
                "session-a",
                &format!("g-{request_id}"),
                &binding,
                &target
            )["kind"],
            "granted"
        );
    }
}

#[test]
fn one_physical_worktree_still_contends_across_sessions() {
    let fixture = Fixture::new(POLICY);
    let root = repo(&fixture.root.join("repo-a"));
    let target = only_target(&write_targets(
        &fixture,
        "session-a",
        "r-a",
        &root.join("tracked.txt"),
    ));

    let owner = bind(&fixture, "session-a", "b-owner", &target);
    assert_eq!(owner["kind"], "bound");

    let contender = bind(&fixture, "session-b", "b-contender", &target);
    assert_eq!(contender["kind"], "denied");
    assert_eq!(contender["state"], "foreign-active");
    assert_eq!(contender["code"], "WORKSPACE_FOREIGN_ACTIVE");
}

#[test]
fn a_forged_or_drifted_target_cannot_bind() {
    let fixture = Fixture::new(POLICY);
    let first = repo(&fixture.root.join("repo-a"));
    let second = repo(&fixture.root.join("repo-b"));
    let target_a = only_target(&write_targets(
        &fixture,
        "session-a",
        "r-a",
        &first.join("tracked.txt"),
    ));

    // A model-shaped root swap keeps the authenticated digest of repository A.
    let mut forged = target_a.clone();
    forged["root"] = json!(second);
    let (code, envelope) = invoke(
        &fixture,
        "bind",
        bind_request("session-a", "b-forged", &forged),
    );
    assert_ne!(code, 0, "envelope={envelope}");
    assert_eq!(envelope["error"]["code"], "workspace-target-invalid");

    // A subdirectory is not a canonical repository root.
    let mut nested = target_a.clone();
    nested["root"] = json!(first.join("nested"));
    fs::create_dir_all(first.join("nested")).expect("nested");
    let (code, envelope) = invoke(
        &fixture,
        "bind",
        bind_request("session-a", "b-nested", &nested),
    );
    assert_ne!(code, 0, "envelope={envelope}");
    assert_eq!(envelope["error"]["code"], "workspace-target-invalid");
}

#[test]
fn an_anchor_bind_is_optional_and_a_non_repository_anchor_needs_no_binding() {
    let fixture = Fixture::new(POLICY);
    let root = repo(&fixture.root.join("repo-a"));
    let plain = fixture.root.join("plain");
    fs::create_dir_all(&plain).expect("plain directory");

    let anchored = ok(
        &fixture,
        "bind",
        anchor_bind_request("session-a", "b-anchor", &root, "startup"),
    );
    assert_eq!(anchored["kind"], "bound");
    assert_eq!(anchored["state"], "owned");

    let unmanaged = ok(
        &fixture,
        "bind",
        anchor_bind_request("session-b", "b-plain", &plain, "startup"),
    );
    assert_eq!(unmanaged["schema_version"], BIND_RESULT);
    assert_eq!(unmanaged["kind"], "not-required");

    // The eager anchor binding is the same durable authority a later lazy
    // acquisition of that repository contends for, so another session's target
    // bind meets it as an active holder.
    let target = only_target(&write_targets(
        &fixture,
        "session-c",
        "r-a",
        &root.join("tracked.txt"),
    ));
    let contender = bind(&fixture, "session-c", "b-contender", &target);
    assert_eq!(contender["kind"], "denied");
    assert_eq!(contender["code"], "WORKSPACE_FOREIGN_ACTIVE");
}

#[test]
fn same_session_resume_recovers_its_own_dirty_target() {
    let fixture = Fixture::new(POLICY);
    let root = repo(&fixture.root.join("repo-a"));
    let target = only_target(&write_targets(
        &fixture,
        "session-a",
        "r-a",
        &root.join("tracked.txt"),
    ));

    let binding = bind(&fixture, "session-a", "b-a", &target);
    assert_eq!(binding["kind"], "bound");
    fs::write(root.join("tracked.txt"), "dirty\n").expect("dirty file");
    assert_eq!(
        release(&fixture, "session-a", "rel-a", &binding)["kind"],
        "released"
    );

    let resumed = ok(
        &fixture,
        "bind",
        json!({
            "schema_version": "agent-hook.workspace-lease.bind.v2",
            "version": 2,
            "request_id": "b-resume",
            "session_id": "session-a",
            "target": target,
            "source": "resume"
        }),
    );
    assert_eq!(resumed["kind"], "bound", "resumed={resumed}");

    let foreign = ok(
        &fixture,
        "bind",
        json!({
            "schema_version": "agent-hook.workspace-lease.bind.v2",
            "version": 2,
            "request_id": "b-foreign",
            "session_id": "session-b",
            "target": target,
            "source": "resume"
        }),
    );
    assert_eq!(foreign["kind"], "denied");
    assert_eq!(foreign["state"], "foreign-active");
}

#[test]
fn mixed_protocol_generations_are_rejected_rather_than_reinterpreted() {
    let fixture = Fixture::new(POLICY);
    let root = repo(&fixture.root.join("repo-a"));
    let target = only_target(&write_targets(
        &fixture,
        "session-a",
        "r-a",
        &root.join("tracked.txt"),
    ));
    let binding = bind(&fixture, "session-a", "b-a", &target);

    // A v2 schema declaring version 1.
    let mut mixed = bind_request("session-a", "b-mixed", &target);
    mixed["version"] = json!(1);
    let (code, envelope) = invoke(&fixture, "bind", mixed);
    assert_ne!(code, 0, "envelope={envelope}");
    assert_eq!(envelope["error"]["code"], "workspace-protocol-unsupported");

    // A v1 schema carrying a v2 target.
    let (code, envelope) = invoke(
        &fixture,
        "bind",
        json!({
            "schema_version": "agent-hook.workspace-lease.bind.v1",
            "version": 1,
            "request_id": "b-v1-target",
            "session_id": "session-a",
            "target": target,
            "source": "startup"
        }),
    );
    assert_ne!(code, 0, "envelope={envelope}");
    assert_eq!(envelope["error"]["code"], "workspace-wire-invalid");

    // A v1 begin must reject the v2-only token as firmly as it rejects a v2
    // target: the field rides on the shared request shape, so ignoring it
    // would widen the otherwise strict v1 wire contract.
    let v1_begin = json!({
        "schema_version": "agent-hook.workspace-lease.begin.v1",
        "version": 1,
        "request_id": "g-v1-token",
        "session_id": "session-a",
        "binding_id": binding["binding_id"],
        "workspace_id": binding["workspace_id"],
        "generation": binding["generation"],
        "binding_state": binding["state"],
        "call_id": "call:g-v1-token",
        "root_call_id": "root:g-v1-token",
        "tool_name": "write",
        "arguments": {"file_path": root.join("tracked.txt"), "content": "next"},
        "nested": false,
        "target_token": "0".repeat(64)
    });
    let (code, envelope) = invoke(&fixture, "begin", v1_begin);
    assert_ne!(code, 0, "envelope={envelope}");
    assert_eq!(envelope["error"]["code"], "workspace-wire-invalid");

    // A v2 begin without an exact target owns no honest coverage claim.
    let mut untargeted = begin_request(
        "session-a",
        "g-untargeted",
        &binding,
        &target,
        BeginCall {
            tool: "write",
            arguments: write_arguments(&target),
            anchor: None,
            token: &Value::Null,
        },
    );
    let fields = untargeted.as_object_mut().expect("object");
    fields.remove("target");
    fields.remove("target_token");
    let (code, envelope) = invoke(&fixture, "begin", untargeted);
    assert_ne!(code, 0, "envelope={envelope}");
    assert_eq!(envelope["error"]["code"], "workspace-wire-invalid");
}

#[test]
fn a_v2_mutation_target_always_receives_a_fence() {
    let fixture = Fixture::new(POLICY);
    let root = repo(&fixture.root.join("repo-a"));
    let target = only_target(&write_targets(
        &fixture,
        "session-a",
        "r-a",
        &root.join("tracked.txt"),
    ));
    let binding = bind(&fixture, "session-a", "b-a", &target);

    // v1 reclassified read-only tool names inside begin. v2 classification is
    // owned by resolve, so a write this boundary classified is fenced on the
    // exact workspace resolve named, whatever the tool is called.
    let granted = begin(&fixture, "session-a", "g-write", &binding, &target);
    assert_eq!(granted["kind"], "granted");

    // begin still never reclassifies the tool name; it requires proof that
    // resolve produced this target for this call. A read-only call is
    // classified `not-required`, so no token exists for it and a begin that
    // names a target anyway fails closed instead of fencing an operation the
    // classifier already excused.
    assert_eq!(
        resolve(
            &fixture,
            "session-a",
            "r-read",
            None,
            "read",
            json!({"file_path": root.join("tracked.txt")})
        )["kind"],
        "not-required"
    );
    let (code, envelope) = invoke(
        &fixture,
        "begin",
        begin_request(
            "session-a",
            "g-read",
            &binding,
            &target,
            BeginCall {
                tool: "read",
                arguments: json!({"file_path": root.join("tracked.txt")}),
                anchor: None,
                token: &Value::Null,
            },
        ),
    );
    assert_ne!(code, 0, "envelope={envelope}");
    assert_eq!(envelope["error"]["code"], "workspace-target-call-mismatch");
}

#[test]
fn v2_operation_replay_and_release_ordering_stay_fenced() {
    let fixture = Fixture::new(POLICY);
    let root = repo(&fixture.root.join("repo-a"));
    let target = only_target(&write_targets(
        &fixture,
        "session-a",
        "r-a",
        &root.join("tracked.txt"),
    ));
    let binding = bind(&fixture, "session-a", "b-a", &target);

    // An active retry of the exact request is idempotent.
    let first = begin(&fixture, "session-a", "g-a", &binding, &target);
    let retry = begin(&fixture, "session-a", "g-a", &binding, &target);
    assert_eq!(first, retry);

    // A release cannot retire authority while an operation lacks an outcome.
    let (code, envelope) = invoke(
        &fixture,
        "release",
        json!({
            "schema_version": "agent-hook.workspace-lease.release.v2",
            "version": 2,
            "request_id": "rel-early",
            "session_id": "session-a",
            "binding_id": binding["binding_id"],
            "workspace_id": binding["workspace_id"],
            "generation": binding["generation"],
            "reason": "agent-disposed"
        }),
    );
    assert_ne!(code, 0, "envelope={envelope}");
    assert_eq!(envelope["error"]["code"], "workspace-release-uncertain");

    assert_eq!(
        complete(&fixture, "session-a", "c-a", &binding, &first, "g-a")["kind"],
        "completed"
    );
    assert_eq!(
        complete(&fixture, "session-a", "c-a", &binding, &first, "g-a")["kind"],
        "duplicate"
    );

    // A terminal operation identity can never be replayed into new authority.
    let replayed = begin(&fixture, "session-a", "g-a", &binding, &target);
    assert_eq!(replayed["kind"], "denied");
    assert_eq!(replayed["code"], "WORKSPACE_OPERATION_REPLAYED");

    assert_eq!(
        release(&fixture, "session-a", "rel-a", &binding)["kind"],
        "released"
    );
    let (code, envelope) = invoke(
        &fixture,
        "begin",
        begin_request(
            "session-a",
            "g-released",
            &binding,
            &target,
            BeginCall {
                tool: "write",
                arguments: write_arguments(&target),
                anchor: None,
                token: &token_for(
                    &fixture,
                    "session-a",
                    "g-released",
                    "write",
                    write_arguments(&target),
                    None,
                    &target["workspace_key"],
                ),
            },
        ),
    );
    assert_eq!(code, 0, "envelope={envelope}");
    assert_eq!(envelope["data"]["kind"], "denied");
    assert_eq!(envelope["data"]["code"], "WORKSPACE_BINDING_RELEASED");
}

#[test]
fn every_v2_binding_names_its_exact_repository_target() {
    let fixture = Fixture::new(POLICY);
    let root = repo(&fixture.root.join("repo-a"));
    let target = only_target(&write_targets(
        &fixture,
        "session-a",
        "r-a",
        &root.join("tracked.txt"),
    ));

    let lazy = bind(&fixture, "session-a", "b-a", &target);
    assert_eq!(lazy["target"], target);

    // The eager anchor binding names the same canonical target, which is what
    // lets a runtime key one authority set by workspace instead of contending
    // with itself. The boundary does not do that keying for it; see
    // `rebinding_a_workspace_this_session_owns_is_denied_like_any_contention`.
    let anchored = ok(
        &fixture,
        "bind",
        anchor_bind_request("session-b", "b-anchor", &root, "startup"),
    );
    assert_eq!(anchored["kind"], "denied");

    let solo = Fixture::new(POLICY);
    let solo_root = repo(&solo.root.join("repo-a"));
    let eager = ok(
        &solo,
        "bind",
        anchor_bind_request("session-a", "b-anchor", &solo_root, "startup"),
    );
    assert_eq!(eager["kind"], "bound");
    let resolved = only_target(&write_targets(
        &solo,
        "session-a",
        "r-a",
        &solo_root.join("tracked.txt"),
    ));
    assert_eq!(eager["target"], resolved);
}

#[test]
fn every_classified_mutation_form_resolves_the_same_target() {
    let fixture = Fixture::new(POLICY);
    let root = repo(&fixture.root.join("repo-a"));
    let file = root.join("tracked.txt");
    let expected = only_target(&write_targets(&fixture, "session-a", "r-write", &file));

    // The classifier admits three declared forms besides `write`. Each is
    // resolved positively here, so narrowing any arm fails a case instead of
    // silently downgrading that mutation to an unfenced operation.
    for (index, (tool, arguments)) in [
        (
            "edit",
            json!({"file_path": file, "old_string": "base", "new_string": "next"}),
        ),
        (
            "str_replace_editor",
            json!({"command": "create", "path": file, "file_text": "next"}),
        ),
        (
            "str_replace_editor",
            json!({"command": "str_replace", "path": file, "old_str": "base", "new_str": "next"}),
        ),
        (
            "str_replace_editor",
            json!({"command": "insert", "path": file, "insert_line": 1, "new_str": "next"}),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let data = resolve(
            &fixture,
            "session-a",
            &format!("r-form-{index}"),
            None,
            tool,
            arguments,
        );
        assert_eq!(data["schema_version"], RESOLVE_RESULT);
        assert_eq!(only_target(&data), expected, "tool={tool} index={index}");
    }
}

#[test]
fn a_governed_commit_resolves_its_anchor_and_fails_closed_without_one() {
    let fixture = Fixture::new(POLICY);
    let root = repo(&fixture.root.join("repo-a"));
    let nested = root.join("nested");
    fs::create_dir_all(&nested).expect("nested");

    // The governed commit declares no path because it has no target to choose:
    // it always commits the canonical live session workspace, so the trusted
    // anchor is the whole proof and converges with a write in that repository.
    let anchored = only_target(&resolve(
        &fixture,
        "session-a",
        "r-commit",
        Some(root.as_path()),
        "runtime_kit_governed_commit",
        json!({}),
    ));
    assert_eq!(anchored["root"], json!(root));
    assert_eq!(
        anchored,
        only_target(&write_targets(
            &fixture,
            "session-a",
            "r-commit-write",
            &root.join("tracked.txt")
        )),
        "a governed commit and a write in one repository must converge"
    );
    assert_eq!(
        only_target(&resolve(
            &fixture,
            "session-a",
            "r-commit-nested",
            Some(nested.as_path()),
            "runtime_kit_governed_commit",
            json!({}),
        )),
        anchored,
        "a subdirectory anchor reduces to the repository top level"
    );

    // Without an anchor there is no admissible target rather than an unscoped
    // commit: the one operation the lease exists to coordinate fails closed.
    let (code, envelope) = invoke(
        &fixture,
        "resolve",
        resolve_request(
            "session-a",
            "r-commit-bare",
            None,
            "runtime_kit_governed_commit",
            json!({}),
        ),
    );
    assert_ne!(code, 0, "envelope={envelope}");
    assert_eq!(envelope["error"]["code"], "workspace-target-unresolvable");
}

#[test]
fn an_artifact_export_resolves_only_its_workspace_destination() {
    let fixture = Fixture::new(POLICY);
    let root = repo(&fixture.root.join("repo-a"));

    // The workspace class writes one file at a path its own schema constrains
    // to be workspace-relative, so it proves the repository exactly.
    let exported = only_target(&resolve(
        &fixture,
        "session-a",
        "r-export",
        Some(root.as_path()),
        "artifact_export",
        json!({"ref": "artifact:a1", "destination": {"class": "workspace", "path": "out/report.md"}}),
    ));
    assert_eq!(
        exported,
        only_target(&write_targets(
            &fixture,
            "session-a",
            "r-export-write",
            &root.join("out/report.md")
        )),
        "an export and a write to one path must converge"
    );

    // The download class writes nothing into a repository.
    let downloaded = resolve(
        &fixture,
        "session-a",
        "r-download",
        Some(root.as_path()),
        "artifact_export",
        json!({"ref": "artifact:a1", "destination": {"class": "download"}}),
    );
    assert_eq!(downloaded["kind"], "not-required", "data={downloaded}");

    // A workspace destination with no anchor cannot be placed.
    let (code, envelope) = invoke(
        &fixture,
        "resolve",
        resolve_request(
            "session-a",
            "r-export-bare",
            None,
            "artifact_export",
            json!({"ref": "artifact:a1", "destination": {"class": "workspace", "path": "out/report.md"}}),
        ),
    );
    assert_ne!(code, 0, "envelope={envelope}");
    assert_eq!(envelope["error"]["code"], "workspace-target-unresolvable");
}

#[test]
fn v2_renew_answers_on_the_v2_result_schema_and_reports_a_lost_binding() {
    let fixture = Fixture::new(POLICY);
    let root = repo(&fixture.root.join("repo-a"));
    let target = only_target(&write_targets(
        &fixture,
        "session-a",
        "r-a",
        &root.join("tracked.txt"),
    ));
    let binding = bind(&fixture, "session-a", "b-a", &target);

    let renewed = ok(
        &fixture,
        "renew",
        renew_request("session-a", "n-a", &binding),
    );
    assert_eq!(renewed["schema_version"], RENEW_RESULT, "data={renewed}");
    assert_eq!(renewed["kind"], "renewed", "data={renewed}");

    // A renewal of a binding this session already released reports the loss on
    // the v2 result schema rather than resurrecting the authority.
    assert_eq!(
        release(&fixture, "session-a", "rel-a", &binding)["kind"],
        "released"
    );
    let lost = ok(
        &fixture,
        "renew",
        renew_request("session-a", "n-lost", &binding),
    );
    assert_eq!(lost["schema_version"], RENEW_RESULT, "data={lost}");
    assert_eq!(lost["kind"], "lost", "data={lost}");
}

#[test]
fn a_v2_bind_rejects_a_target_and_an_anchor_together() {
    let fixture = Fixture::new(POLICY);
    let root = repo(&fixture.root.join("repo-a"));
    let target = only_target(&write_targets(
        &fixture,
        "session-a",
        "r-a",
        &root.join("tracked.txt"),
    ));

    // Authority comes from exactly one source. Admitting both would let a
    // forged cwd ride along beside a valid target and be silently ignored.
    let mut request = bind_request("session-a", "b-both", &target);
    request["cwd"] = json!(root);
    let (code, envelope) = invoke(&fixture, "bind", request);
    assert_ne!(code, 0, "envelope={envelope}");
    assert_eq!(envelope["error"]["code"], "workspace-wire-invalid");
}

#[test]
fn resolve_rejects_a_relative_anchor_and_an_unusable_target_path() {
    let fixture = Fixture::new(POLICY);
    let root = repo(&fixture.root.join("repo-a"));

    // The anchor is what every relative target is joined against, so a
    // relative anchor would resolve targets against the boundary's own process
    // directory and could fence a different repository than the one written.
    let mut relative = resolve_request(
        "session-a",
        "r-rel",
        None,
        "write",
        json!({"file_path": "tracked.txt", "content": "next"}),
    );
    relative["anchor_cwd"] = json!("relative/anchor");
    let (code, envelope) = invoke(&fixture, "resolve", relative);
    assert_ne!(code, 0, "envelope={envelope}");
    assert_eq!(envelope["error"]["code"], "workspace-cwd-invalid");

    for (label, arguments) in [
        ("empty", json!({"file_path": "", "content": "next"})),
        (
            "nul",
            json!({"file_path": "trac\u{0}ked.txt", "content": "next"}),
        ),
        ("absent", json!({"content": "next"})),
    ] {
        let (code, envelope) = invoke(
            &fixture,
            "resolve",
            resolve_request(
                "session-a",
                &format!("r-bad-{label}"),
                Some(root.as_path()),
                "write",
                arguments,
            ),
        );
        assert_ne!(code, 0, "label={label} envelope={envelope}");
        assert_eq!(
            envelope["error"]["code"], "workspace-target-unresolvable",
            "label={label}"
        );
    }
}

#[test]
fn rebinding_a_workspace_this_session_owns_is_denied_like_any_contention() {
    let fixture = Fixture::new(POLICY);
    let root = repo(&fixture.root.join("repo-a"));

    let anchored = ok(
        &fixture,
        "bind",
        anchor_bind_request("session-a", "b-anchor", &root, "startup"),
    );
    assert_eq!(anchored["kind"], "bound");
    let target = only_target(&write_targets(
        &fixture,
        "session-a",
        "r-a",
        &root.join("tracked.txt"),
    ));

    // The anchor bind already names the workspace key the lazy target resolves
    // to, and that key is what the runtime must converge on. Convergence is the
    // runtime's obligation, not a boundary guarantee: rebinding a workspace
    // this session already owns is denied exactly like a foreign holder, and
    // the denial cannot be told apart from genuine contention.
    assert_eq!(anchored["target"]["workspace_key"], target["workspace_key"]);
    let denied = bind(&fixture, "session-a", "b-again", &target);
    assert_eq!(denied["kind"], "denied", "data={denied}");
    assert_eq!(denied["code"], "WORKSPACE_FOREIGN_ACTIVE", "data={denied}");
}

#[test]
fn concurrent_first_use_resolution_never_distrusts_the_state_directory() {
    let fixture = Fixture::new(POLICY);
    let root = repo(&fixture.root.join("repo-a"));
    let file = root.join("tracked.txt");

    // v2 makes every classified tool call a first-use creator of the private
    // state root, where v1 created it once per session start. Creating the
    // directory and only then chmodding it would let one process observe
    // another's umask-derived mode and reject it as untrusted, which denies the
    // tool call outright rather than retrying a bind.
    let fixture = &fixture;
    std::thread::scope(|scope| {
        let handles = (0..8)
            .map(|index| {
                let request = resolve_request(
                    "session-a",
                    &format!("r-race-{index}"),
                    None,
                    "write",
                    json!({"file_path": file, "content": "next"}),
                );
                scope.spawn(move || {
                    let output = fixture.run(
                        &["workspace-lease", "resolve", "--format", "json"],
                        Some(&request.to_string()),
                    );
                    (output.code, output.stdout_json())
                })
            })
            .collect::<Vec<_>>();
        for (index, handle) in handles.into_iter().enumerate() {
            let (code, envelope) = handle.join().expect("resolve thread");
            assert_eq!(code, 0, "index={index} envelope={envelope}");
            assert_eq!(envelope["data"]["kind"], "targets", "index={index}");
        }
    });
}

#[test]
fn a_v2_begin_is_bound_to_the_call_its_target_was_classified_from() {
    let fixture = Fixture::new(POLICY);
    let root_a = repo(&fixture.root.join("repo-a"));
    let root_b = repo(&fixture.root.join("repo-b"));

    let resolved_a = write_targets(&fixture, "session-a", "r-a", &root_a.join("tracked.txt"));
    let target_a = only_target(&resolved_a);
    let token_a = only_token(&resolved_a);
    let target_b = only_target(&write_targets(
        &fixture,
        "session-a",
        "r-b",
        &root_b.join("tracked.txt"),
    ));
    assert_ne!(target_a["workspace_key"], target_b["workspace_key"]);

    let binding_a = bind(&fixture, "session-a", "b-a", &target_a);
    assert_eq!(binding_a["kind"], "bound");

    // The attack this closes: fence the repository the caller legitimately
    // holds while the call's own arguments mutate a different one. Repository A
    // is named and bound; the execution writes into repository B.
    let against_b = json!({"file_path": root_b.join("tracked.txt"), "content": "next"});
    let honest_b_token = token_for(
        &fixture,
        "session-a",
        "g-cross-call",
        "write",
        against_b.clone(),
        None,
        &target_b["workspace_key"],
    );
    let (code, envelope) = invoke(
        &fixture,
        "begin",
        begin_request(
            "session-a",
            "g-cross-call",
            &binding_a,
            &target_a,
            BeginCall {
                tool: "write",
                arguments: against_b.clone(),
                anchor: None,
                token: &honest_b_token,
            },
        ),
    );
    assert_ne!(code, 0, "envelope={envelope}");
    assert_eq!(
        envelope["error"]["code"], "workspace-target-call-mismatch",
        "envelope={envelope}"
    );

    // The rejection is not the identity verdict: repository A is exactly the
    // workspace the binding owns, so the two failures stay distinguishable.
    assert_ne!(envelope["error"]["code"], "workspace-target-invalid");

    // A token minted for repository A cannot launder a call that writes into
    // repository B either: the token is keyed over the call facts too.
    let (code, envelope) = invoke(
        &fixture,
        "begin",
        begin_request(
            "session-a",
            "g-cross-token",
            &binding_a,
            &target_a,
            BeginCall {
                tool: "write",
                arguments: against_b,
                anchor: None,
                token: &token_a,
            },
        ),
    );
    assert_ne!(code, 0, "envelope={envelope}");
    assert_eq!(
        envelope["error"]["code"], "workspace-target-call-mismatch",
        "envelope={envelope}"
    );

    // The honest execution against repository A is still granted.
    assert_eq!(
        begin(&fixture, "session-a", "g-a", &binding_a, &target_a)["kind"],
        "granted"
    );
}

#[test]
fn a_v2_begin_rejects_a_missing_forged_or_replayed_target_token() {
    let fixture = Fixture::new(POLICY);
    let root = repo(&fixture.root.join("repo-a"));
    let target = only_target(&write_targets(
        &fixture,
        "session-a",
        "r-a",
        &root.join("tracked.txt"),
    ));
    let binding = bind(&fixture, "session-a", "b-a", &target);
    let arguments = write_arguments(&target);

    // A token minted for a different call of the same tool against the same
    // repository is still the wrong token: the call facts differ.
    let other_call_token = token_for(
        &fixture,
        "session-a",
        "r-other",
        "write",
        arguments.clone(),
        None,
        &target["workspace_key"],
    );

    // So is a token minted for a different tool that classifies to the same
    // repository, because the tool name is part of the call facts.
    let other_tool_token = token_for(
        &fixture,
        "session-a",
        "g-token",
        "edit",
        arguments.clone(),
        None,
        &target["workspace_key"],
    );

    for (label, token) in [
        ("missing", Value::Null),
        ("empty", json!("")),
        ("malformed", json!("not-a-digest")),
        ("wrong-length", json!("abcdef")),
        ("forged", json!("0".repeat(64))),
        ("other-call", other_call_token),
        ("other-tool", other_tool_token),
    ] {
        let (code, envelope) = invoke(
            &fixture,
            "begin",
            begin_request(
                "session-a",
                "g-token",
                &binding,
                &target,
                BeginCall {
                    tool: "write",
                    arguments: arguments.clone(),
                    anchor: None,
                    token: &token,
                },
            ),
        );
        assert_ne!(code, 0, "label={label} envelope={envelope}");
        assert_eq!(
            envelope["error"]["code"], "workspace-target-call-mismatch",
            "label={label} envelope={envelope}"
        );
    }
}

#[test]
fn a_v2_bind_needs_no_target_token() {
    let fixture = Fixture::new(POLICY);
    let root = repo(&fixture.root.join("repo-a"));
    let resolved = write_targets(&fixture, "session-a", "r-a", &root.join("tracked.txt"));
    let target = only_target(&resolved);

    // bind authenticates identity by rederiving the workspace digest from the
    // live layout, so it depends on no call facts and accepts no token.
    let binding = bind(&fixture, "session-a", "b-a", &target);
    assert_eq!(binding["kind"], "bound");
    assert_eq!(binding["target"], target);

    let mut tokened = bind_request("session-a", "b-token", &target);
    tokened["target"]["token"] = only_token(&resolved);
    let (code, envelope) = invoke(&fixture, "bind", tokened);
    assert_ne!(code, 0, "envelope={envelope}");
    assert_eq!(envelope["error"]["code"], "workspace-wire-invalid");
}

#[test]
fn a_v2_begin_token_binds_the_anchor_the_target_was_derived_from() {
    let fixture = Fixture::new(POLICY);
    let root_a = repo(&fixture.root.join("repo-a"));
    let root_b = repo(&fixture.root.join("repo-b"));

    // A governed commit names no path at all: its target is derived entirely
    // from the classification anchor. One set of call facts therefore resolves
    // to a different repository under a different anchor, so a token that did
    // not bind the anchor would authenticate either target for the same call.
    let commit_arguments = json!({});
    let resolved_a = resolve(
        &fixture,
        "session-a",
        "r-anchor",
        Some(&root_a),
        "runtime_kit_governed_commit",
        commit_arguments.clone(),
    );
    let target_a = only_target(&resolved_a);
    let token_a = only_token(&resolved_a);
    let resolved_b = resolve(
        &fixture,
        "session-a",
        "r-anchor",
        Some(&root_b),
        "runtime_kit_governed_commit",
        commit_arguments.clone(),
    );
    let target_b = only_target(&resolved_b);
    let token_b = only_token(&resolved_b);
    assert_ne!(target_a["workspace_key"], target_b["workspace_key"]);
    assert_ne!(token_a, token_b);

    let binding_a = bind(&fixture, "session-a", "b-a", &target_a);
    assert_eq!(binding_a["kind"], "bound");

    // Repository A is named and bound, and the call facts are identical, but
    // the execution's own anchor is repository B.
    let (code, envelope) = invoke(
        &fixture,
        "begin",
        begin_request(
            "session-a",
            "r-anchor",
            &binding_a,
            &target_a,
            BeginCall {
                tool: "runtime_kit_governed_commit",
                arguments: commit_arguments.clone(),
                anchor: Some(&root_b),
                token: &token_a,
            },
        ),
    );
    assert_ne!(code, 0, "envelope={envelope}");
    assert_eq!(
        envelope["error"]["code"], "workspace-target-call-mismatch",
        "envelope={envelope}"
    );

    // Repository B's honest token does not authenticate repository A either.
    let (code, envelope) = invoke(
        &fixture,
        "begin",
        begin_request(
            "session-a",
            "r-anchor",
            &binding_a,
            &target_a,
            BeginCall {
                tool: "runtime_kit_governed_commit",
                arguments: commit_arguments.clone(),
                anchor: Some(&root_b),
                token: &token_b,
            },
        ),
    );
    assert_ne!(code, 0, "envelope={envelope}");
    assert_eq!(
        envelope["error"]["code"], "workspace-target-call-mismatch",
        "envelope={envelope}"
    );

    // Omitting the anchor entirely is a different classification too.
    let (code, envelope) = invoke(
        &fixture,
        "begin",
        begin_request(
            "session-a",
            "r-anchor",
            &binding_a,
            &target_a,
            BeginCall {
                tool: "runtime_kit_governed_commit",
                arguments: commit_arguments.clone(),
                anchor: None,
                token: &token_a,
            },
        ),
    );
    assert_ne!(code, 0, "envelope={envelope}");
    assert_eq!(
        envelope["error"]["code"], "workspace-target-call-mismatch",
        "envelope={envelope}"
    );

    // The honest execution against the anchor it was classified from is
    // granted.
    let granted = ok(
        &fixture,
        "begin",
        begin_request(
            "session-a",
            "r-anchor",
            &binding_a,
            &target_a,
            BeginCall {
                tool: "runtime_kit_governed_commit",
                arguments: commit_arguments,
                anchor: Some(&root_a),
                token: &token_a,
            },
        ),
    );
    assert_eq!(granted["kind"], "granted", "granted={granted}");
}

#[test]
fn a_v2_begin_token_ignores_argument_key_order() {
    let fixture = Fixture::new(POLICY);
    let root = repo(&fixture.root.join("repo-a"));
    let tracked = root.join("tracked.txt");

    // The runtime resolves and begins in separate process invocations, so the
    // adapter may reserialize `arguments` between them. Under a workspace build
    // serde_json preserves insertion order, so an order-sensitive token would
    // deny every fenced mutation rather than merely miss an idempotency match.
    let resolved = resolve(
        &fixture,
        "session-a",
        "r-order",
        None,
        "write",
        json!({"content": "next", "file_path": tracked}),
    );
    let target = only_target(&resolved);
    let token = only_token(&resolved);
    let binding = bind(&fixture, "session-a", "b-a", &target);

    let granted = ok(
        &fixture,
        "begin",
        begin_request(
            "session-a",
            "r-order",
            &binding,
            &target,
            BeginCall {
                tool: "write",
                arguments: json!({"file_path": tracked, "content": "next"}),
                anchor: None,
                token: &token,
            },
        ),
    );
    assert_eq!(granted["kind"], "granted", "granted={granted}");
}
