#[allow(dead_code)]
#[path = "../src/debug/mod.rs"]
mod debug;

use debug::schema::{ArtifactStatus, BUNDLE_MANIFEST_FILE_NAME, BUNDLE_MANIFEST_SCHEMA_VERSION};
use nils_test_support::git::{self, InitRepoOptions};
use nils_test_support::{prepend_path, CwdGuard, EnvGuard, GlobalStateLock, StubBinDir};
use pretty_assertions::assert_eq;
use serde_json::Value;
use std::path::Path;

fn install_success_stubs(stub: &StubBinDir) {
    stub.write_exe(
        "macos-agent",
        "#!/bin/sh\necho '{\"ok\":true,\"command\":\"preflight\"}'\n",
    );
    stub.write_exe("screen-record", "#!/bin/sh\necho 'preflight ok'\n");
    stub.write_exe(
        "image-processing",
        "#!/bin/sh\necho 'image-processing help'\n",
    );
}

fn install_partial_failure_stubs(stub: &StubBinDir) {
    stub.write_exe(
        "macos-agent",
        "#!/bin/sh\necho '{\"ok\":true,\"command\":\"preflight\"}'\n",
    );
    stub.write_exe("screen-record", "#!/bin/sh\necho 'preflight ok'\n");
    stub.write_exe(
        "image-processing",
        "#!/bin/sh\necho 'magick is missing' 1>&2\nexit 7\n",
    );
}

fn artifact_ids(manifest: &debug::schema::BundleManifest) -> Vec<String> {
    manifest
        .artifacts
        .iter()
        .map(|artifact| artifact.id.clone())
        .collect::<Vec<_>>()
}

fn artifact_paths(manifest: &debug::schema::BundleManifest) -> Vec<String> {
    manifest
        .artifacts
        .iter()
        .map(|artifact| artifact.path.clone())
        .collect::<Vec<_>>()
}

fn read_manifest_json(output_dir: &Path) -> Value {
    let raw = std::fs::read_to_string(output_dir.join(BUNDLE_MANIFEST_FILE_NAME))
        .expect("manifest file should be readable");
    serde_json::from_str(&raw).expect("manifest file should be json")
}

#[test]
fn debug_bundle_manifest_is_versioned_and_always_emitted() {
    let lock = GlobalStateLock::new();
    let stub = StubBinDir::new();
    install_success_stubs(&stub);
    let repo = git::init_repo_with(
        InitRepoOptions::new()
            .with_branch("main")
            .with_initial_commit(),
    );

    let _path = prepend_path(&lock, stub.path());
    let _cwd = CwdGuard::set(&lock, repo.path()).expect("set cwd");
    let output_dir = repo.path().join("bundle-success");

    let manifest = debug::bundle::collect_bundle(&output_dir).expect("collect debug bundle");

    assert_eq!(manifest.schema_version, BUNDLE_MANIFEST_SCHEMA_VERSION);
    assert_eq!(manifest.manifest_version, 1);
    assert!(output_dir.join(BUNDLE_MANIFEST_FILE_NAME).is_file());
    assert_eq!(manifest.summary.total_artifacts, 4);
    assert_eq!(manifest.summary.failed, 0);
    assert_eq!(manifest.partial_failure, false);
    assert_eq!(
        artifact_ids(&manifest),
        vec![
            debug::sources::git_context::ARTIFACT_ID.to_string(),
            debug::sources::macos_agent::ARTIFACT_ID.to_string(),
            debug::sources::screen_record::ARTIFACT_ID.to_string(),
            debug::sources::image_processing::ARTIFACT_ID.to_string(),
        ]
    );

    let persisted = read_manifest_json(&output_dir);
    assert_eq!(persisted["schema_version"], BUNDLE_MANIFEST_SCHEMA_VERSION);
    assert_eq!(persisted["manifest_version"], 1);
}

#[test]
fn debug_bundle_partial_failures_keep_successful_artifact_refs() {
    let lock = GlobalStateLock::new();
    let stub = StubBinDir::new();
    install_partial_failure_stubs(&stub);
    let repo = git::init_repo_with(
        InitRepoOptions::new()
            .with_branch("main")
            .with_initial_commit(),
    );

    let _path = prepend_path(&lock, stub.path());
    let _cwd = CwdGuard::set(&lock, repo.path()).expect("set cwd");
    let output_dir = repo.path().join("bundle-partial");

    let manifest = debug::bundle::collect_bundle(&output_dir).expect("collect debug bundle");

    assert!(manifest.partial_failure);
    assert_eq!(manifest.summary.total_artifacts, 4);
    assert_eq!(manifest.summary.failed, 1);

    let git_context = manifest
        .artifacts
        .iter()
        .find(|artifact| artifact.id == debug::sources::git_context::ARTIFACT_ID)
        .expect("git context artifact should exist");
    assert_eq!(git_context.status, ArtifactStatus::Collected);
    assert_eq!(
        git_context.path,
        debug::sources::git_context::ARTIFACT_RELATIVE_PATH
    );

    let image_processing = manifest
        .artifacts
        .iter()
        .find(|artifact| artifact.id == debug::sources::image_processing::ARTIFACT_ID)
        .expect("image processing artifact should exist");
    assert_eq!(image_processing.status, ArtifactStatus::Failed);
    assert!(image_processing
        .error
        .as_deref()
        .unwrap_or_default()
        .contains("image-processing"));

    let persisted = read_manifest_json(&output_dir);
    assert_eq!(persisted["partial_failure"], true);
    let artifact_array = persisted["artifacts"]
        .as_array()
        .expect("artifacts should be an array");
    assert!(
        artifact_array.iter().any(|artifact| {
            artifact["id"] == debug::sources::git_context::ARTIFACT_ID
                && artifact["status"] == "collected"
        }),
        "successful artifact reference should still be present"
    );
    assert!(
        artifact_array.iter().any(|artifact| {
            artifact["id"] == debug::sources::image_processing::ARTIFACT_ID
                && artifact["status"] == "failed"
        }),
        "failed artifact entry should be present"
    );
}

#[test]
fn debug_bundle_artifact_layout_is_deterministic_under_output_dir() {
    let lock = GlobalStateLock::new();
    let stub = StubBinDir::new();
    install_success_stubs(&stub);
    let repo = git::init_repo_with(
        InitRepoOptions::new()
            .with_branch("main")
            .with_initial_commit(),
    );

    let _path = prepend_path(&lock, stub.path());
    let _cwd = CwdGuard::set(&lock, repo.path()).expect("set cwd");
    let output_dir = repo.path().join("bundle-deterministic");

    let first = debug::bundle::collect_bundle(&output_dir).expect("collect first bundle");
    let second = debug::bundle::collect_bundle(&output_dir).expect("collect second bundle");
    assert_eq!(artifact_paths(&first), artifact_paths(&second));
    assert_eq!(
        artifact_paths(&first),
        vec![
            debug::sources::git_context::ARTIFACT_RELATIVE_PATH.to_string(),
            debug::sources::macos_agent::ARTIFACT_RELATIVE_PATH.to_string(),
            debug::sources::screen_record::ARTIFACT_RELATIVE_PATH.to_string(),
            debug::sources::image_processing::ARTIFACT_RELATIVE_PATH.to_string(),
        ]
    );
    for relative_path in artifact_paths(&first) {
        assert!(
            output_dir.join(relative_path).is_file(),
            "artifact file should exist under output dir"
        );
    }

    let codex_home = repo.path().join("codex-home");
    std::fs::create_dir_all(&codex_home).expect("create codex home");
    let codex_home_str = codex_home.to_string_lossy().to_string();
    let _codex_home = EnvGuard::set(&lock, "CODEX_HOME", &codex_home_str);

    assert_eq!(
        debug::bundle::resolve_output_dir(None),
        codex_home.join("out").join("agentctl-debug-bundle")
    );
    assert_eq!(
        debug::bundle::resolve_output_dir(Some(output_dir.as_path())),
        output_dir
    );
}
