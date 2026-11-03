use super::*;
use nils_test_support::{EnvGuard, GlobalStateLock};
use pretty_assertions::assert_eq;
use serde_json::json;
use std::path::{Path, PathBuf};

pub(crate) fn record(root: &Path, provider: &str) -> SessionRecord {
    serde_json::from_value(json!({
        "schema_version":"agent-session.session.v1","id":"synthetic","agent":provider,"mode":"interactive",
        "cwd":"/workspace/example","tmux_session":"synthetic","created_at":"2026-01-02T00:00:00Z","updated_at":"2026-01-02T00:00:00Z",
        "provider_resume":{"provider":provider,"session_id":"conversation","captured_at":"2026-01-02T00:00:00Z","capture_method":"fixture","resume_args":[]},
        "runtime":{"kind":"tmux","tmux_session":"synthetic","generation":1,"started_at":"2026-01-02T00:00:00Z","launch_id":"runtime","agent_profile_provider_config_dir":root,"agent_profile":"profile"}
    })).unwrap()
}

pub(crate) fn history(root: &Path, provider: &str) -> PathBuf {
    let path = root.join(if provider == "claude" {
        "projects/project/conversation.jsonl"
    } else {
        "sessions/rollout.jsonl"
    });
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let header = if provider == "claude" {
        json!({"type":"user","sessionId":"conversation","cwd":"/workspace/example","text":"TEXT_CANARY"})
    } else {
        json!({"type":"session_meta","timestamp":"2020-01-01T00:00:00Z","payload":{"id":"conversation","cwd":"/workspace/example","source":"cli","timestamp":"2026-01-01T00:00:00Z"},"extra":"FIELD_CANARY"})
    };
    fs::write(&path, format!("{header}\n")).unwrap();
    path
}

fn context(root: &Path, record: &SessionRecord) -> CliContext {
    let context = CliContext {
        state_dir: root.join("state"),
        host: None,
    };
    crate::write_session_record(&context, record).unwrap();
    context
}

fn now() -> jiff::Timestamp {
    "2026-01-03T00:00:00Z".parse().unwrap()
}
fn collect(context: &CliContext, record: &SessionRecord) -> SessionMetrics {
    collect_at(context, record, now(), || {})
}

#[test]
fn both_providers_isolate_roots_and_expose_different_managed_and_provider_ages() {
    for provider in ["claude", "codex"] {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("config");
        history(&root, provider);
        let record = record(&root, provider);
        let context = context(tmp.path(), &record);
        let metrics = collect(&context, &record);
        assert_eq!(metrics.managed_age_seconds, Metric::known(86400));
        assert_eq!(
            metrics.conversation_age_seconds,
            if provider == "codex" {
                Metric::known(172800)
            } else {
                Metric::unknown(Reason::ProviderCreationUnavailable)
            }
        );
        assert_eq!(metrics.transcript_lines, Metric::known(1));
        assert_eq!(metrics.completed_compactions, Metric::known(0));
        let other = tmp.path().join("other");
        let path = history(&other, provider);
        let second_content = format!(
            "{}{}\n",
            fs::read_to_string(&path).unwrap(),
            serde_json::json!({"type":"user"})
        );
        fs::write(&path, second_content).unwrap();
        let mut second = record.clone();
        second.id = "second-managed".into();
        second
            .runtime
            .as_mut()
            .unwrap()
            .extra
            .insert("agent_profile".into(), json!("second-profile"));
        second
            .runtime
            .as_mut()
            .unwrap()
            .extra
            .insert("agent_profile_provider_config_dir".into(), json!(other));
        crate::write_session_record(&context, &second).unwrap();
        assert_eq!(
            collect(&context, &second).transcript_lines,
            Metric::known(2)
        );
        assert_eq!(
            collect(&context, &record).transcript_lines,
            Metric::known(1)
        );
        fs::write(path, b"wrong header\n").unwrap();
        assert_eq!(
            collect(&context, &second).transcript_bytes,
            Metric::unknown(Reason::SourceUnavailable)
        );
    }
}

#[test]
fn invalid_future_managed_and_provider_timestamps_are_unknown_without_clamping() {
    let tmp = tempfile::tempdir().unwrap();
    let path = history(tmp.path(), "codex");
    let mut record = record(tmp.path(), "codex");
    let context = context(tmp.path(), &record);
    for timestamp in ["invalid", "2027-01-01T00:00:00Z"] {
        record.created_at = timestamp.into();
        crate::write_session_record(&context, &record).unwrap();
        let metrics = collect(&context, &record);
        assert_eq!(
            metrics.managed_age_seconds,
            Metric::unknown(Reason::InvalidTimestamp)
        );
        assert_eq!(metrics.managed_created_at, None);
    }
    let bytes = fs::read_to_string(&path)
        .unwrap()
        .replace("2026-01-01", "2027-01-01");
    fs::write(path, bytes).unwrap();
    assert_eq!(
        collect(&context, &record).conversation_age_seconds,
        Metric::unknown(Reason::InvalidTimestamp)
    );
}

#[test]
fn duplicate_truncated_wrong_header_sidechain_and_noninteractive_sources_are_unknown() {
    let lock = GlobalStateLock::new();
    for provider in ["claude", "codex"] {
        let tmp = tempfile::tempdir().unwrap();
        let path = history(tmp.path(), provider);
        let record = record(tmp.path(), provider);
        let context = context(tmp.path(), &record);
        let original = fs::read_to_string(&path).unwrap();
        for invalid in [
            original.replace("conversation", "foreign"),
            if provider == "claude" {
                original.replace("\"type\":", "\"isSidechain\":true,\"type\":")
            } else {
                original.replace("\"cli\"", "\"exec\"")
            },
        ] {
            fs::write(&path, invalid).unwrap();
            assert_eq!(
                collect(&context, &record).transcript_lines,
                Metric::unknown(Reason::SourceUnavailable)
            );
        }
        fs::write(&path, original).unwrap();
        fs::write(path.with_file_name("unrelated.jsonl"), "{}\n").unwrap();
        let variable = if provider == "claude" {
            "AGENT_SESSION_CLAUDE_RESUME_SCAN_MAX_ENTRIES"
        } else {
            "AGENT_SESSION_CODEX_RESUME_SCAN_MAX_ENTRIES"
        };
        let guard = EnvGuard::set(&lock, variable, "1");
        assert_eq!(
            collect(&context, &record).transcript_lines,
            Metric::unknown(Reason::SourceUnavailable)
        );
        drop(guard);
        let duplicate = path.parent().unwrap().join("duplicate.jsonl");
        if provider == "claude" {
            let duplicate = tmp.path().join("projects/duplicate/conversation.jsonl");
            fs::create_dir_all(duplicate.parent().unwrap()).unwrap();
            fs::copy(&path, duplicate).unwrap();
        } else {
            fs::copy(&path, duplicate).unwrap();
        }
        assert_eq!(
            collect(&context, &record).transcript_bytes,
            Metric::unknown(Reason::SourceUnavailable)
        );
    }
}

#[test]
fn unsafe_missing_and_replaced_sources_never_reuse_stale_measurements() {
    for provider in ["claude", "codex"] {
        let tmp = tempfile::tempdir().unwrap();
        let path = history(tmp.path(), provider);
        let record = record(tmp.path(), provider);
        let context = context(tmp.path(), &record);
        assert!(collect(&context, &record).transcript_bytes.value.is_some());
        let retained = tmp.path().join("retained");
        fs::rename(&path, &retained).unwrap();
        assert_eq!(
            collect(&context, &record).transcript_bytes,
            Metric::unknown(Reason::SourceUnavailable)
        );
        std::os::unix::fs::symlink(&retained, &path).unwrap();
        assert_eq!(
            collect(&context, &record).transcript_bytes,
            Metric::unknown(Reason::SourceUnavailable)
        );
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert_eq!(
            collect(&context, &record).transcript_bytes,
            Metric::unknown(Reason::SourceUnavailable)
        );
    }
}

#[test]
fn native_transition_and_record_identity_changes_invalidate_observations_without_repair() {
    let tmp = tempfile::tempdir().unwrap();
    history(tmp.path(), "claude");
    let record = record(tmp.path(), "claude");
    let context = context(tmp.path(), &record);
    crate::conversation::retain_observation(&context, &record, "other-conversation").unwrap();
    assert_eq!(
        collect(&context, &record).transcript_bytes,
        Metric::unknown(Reason::IdentityChanged)
    );
    assert_eq!(
        crate::load_session_record(&context, &record.id)
            .unwrap()
            .provider_resume
            .unwrap()
            .session_id,
        "conversation"
    );
    for field in [
        "created_at",
        "launch_id",
        "generation",
        "profile",
        "root",
        "conversation",
    ] {
        fs::remove_file(
            crate::session_dir(&context, &record.id).join("provider-conversation.json"),
        )
        .ok();
        crate::write_session_record(&context, &record).unwrap();
        let result = collect_at(&context, &record, now(), || {
            let mut next = record.clone();
            match field {
                "created_at" => next.created_at = "2026-01-01T00:00:00Z".into(),
                "launch_id" => next.runtime.as_mut().unwrap().launch_id = "next".into(),
                "generation" => next.runtime.as_mut().unwrap().generation += 1,
                "profile" => {
                    next.runtime
                        .as_mut()
                        .unwrap()
                        .extra
                        .insert("agent_profile".into(), json!("other"));
                }
                "root" => {
                    next.runtime.as_mut().unwrap().extra.insert(
                        "agent_profile_provider_config_dir".into(),
                        json!(tmp.path().join("other")),
                    );
                }
                _ => next.provider_resume.as_mut().unwrap().session_id = "next-conversation".into(),
            }
            crate::write_session_record(&context, &next).unwrap();
        });
        assert_eq!(
            result.transcript_lines,
            Metric::unknown(Reason::IdentityChanged),
            "{field}"
        );
    }
}

#[test]
fn concurrent_append_rewrite_truncation_and_replacement_invalidate_all_file_facts() {
    for mutation in ["append", "rewrite", "truncate", "replace"] {
        let tmp = tempfile::tempdir().unwrap();
        let path = history(tmp.path(), "codex");
        let record = record(tmp.path(), "codex");
        let context = context(tmp.path(), &record);
        let result = collect_at(&context, &record, now(), || match mutation {
            "append" => {
                use std::io::Write;
                fs::OpenOptions::new()
                    .append(true)
                    .open(&path)
                    .unwrap()
                    .write_all(b"{}\n")
                    .unwrap();
            }
            "truncate" => fs::write(&path, b"").unwrap(),
            "rewrite" => {
                let length = fs::metadata(&path).unwrap().len() as usize;
                fs::write(&path, vec![b' '; length]).unwrap();
            }
            _ => {
                let replacement = path.with_extension("replacement");
                fs::copy(&path, &replacement).unwrap();
                fs::rename(replacement, &path).unwrap();
            }
        });
        assert_eq!(
            result.transcript_bytes,
            Metric::unknown(Reason::IdentityChanged),
            "{mutation}"
        );
        assert_eq!(
            result.transcript_lines,
            Metric::unknown(Reason::IdentityChanged)
        );
        assert_eq!(
            result.conversation_age_seconds,
            Metric::unknown(Reason::IdentityChanged)
        );
        assert_eq!(result.managed_age_seconds, Metric::known(86400));
    }
}

#[test]
fn cap_exceeded_keeps_independent_bytes_and_creation_but_never_partial_counts() {
    let tmp = tempfile::tempdir().unwrap();
    let path = history(tmp.path(), "codex");
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(counts::MAX_COUNT_BYTES + 1)
        .unwrap();
    let record = record(tmp.path(), "codex");
    let context = context(tmp.path(), &record);
    let metrics = collect(&context, &record);
    assert_eq!(
        metrics.transcript_bytes,
        Metric::known(counts::MAX_COUNT_BYTES + 1)
    );
    assert_eq!(
        metrics.transcript_lines,
        Metric::unknown(Reason::ReadBudgetExceeded)
    );
    assert_eq!(
        metrics.completed_compactions,
        Metric::unknown(Reason::ReadBudgetExceeded)
    );
    assert_eq!(metrics.conversation_age_seconds, Metric::known(172800));
    assert!(!serde_json::to_string(&metrics).unwrap().contains("CANARY"));
}

#[test]
fn timestamp_normalization_and_fractional_age_use_whole_elapsed_seconds() {
    let tmp = tempfile::tempdir().unwrap();
    let mut record = record(tmp.path(), "claude");
    record.created_at = "2026-01-02T01:00:00.500+01:00".into();
    let context = context(tmp.path(), &record);
    let metrics = collect(&context, &record);
    assert_eq!(
        metrics.managed_created_at.as_deref(),
        Some("2026-01-02T00:00:00.5Z")
    );
    assert_eq!(metrics.managed_age_seconds, Metric::known(86399));
}

#[test]
fn invalid_oversized_and_nonregular_native_receipts_cannot_authorize_stale_content() {
    let tmp = tempfile::tempdir().unwrap();
    history(tmp.path(), "claude");
    let record = record(tmp.path(), "claude");
    let context = context(tmp.path(), &record);
    let receipt = crate::session_dir(&context, &record.id).join("provider-conversation.json");
    for bytes in [
        b"INVALID_RECEIPT_CANARY".to_vec(),
        vec![b' '; 16 * 1024 + 1],
    ] {
        fs::write(&receipt, bytes).unwrap();
        let metrics = collect(&context, &record);
        assert_eq!(
            metrics.transcript_bytes,
            Metric::unknown(Reason::IdentityChanged)
        );
        assert!(!serde_json::to_string(&metrics).unwrap().contains("CANARY"));
    }
    fs::remove_file(&receipt).unwrap();
    fs::create_dir(&receipt).unwrap();
    assert_eq!(
        collect(&context, &record).transcript_lines,
        Metric::unknown(Reason::IdentityChanged)
    );
}
