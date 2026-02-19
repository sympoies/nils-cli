use std::path::PathBuf;

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn adapter_depends_on_claude_core_runtime_modules() {
    let adapter_src = std::fs::read_to_string(manifest_dir().join("src/adapter.rs"))
        .expect("read adapter source");

    assert!(
        adapter_src.contains("use claude_core::config"),
        "adapter should import config from claude_core"
    );
    assert!(
        adapter_src.contains("use claude_core::exec"),
        "adapter should import exec from claude_core"
    );
    assert!(
        !adapter_src.contains("use crate::client"),
        "adapter must not import local runtime client module"
    );
    assert!(
        !adapter_src.contains("use crate::config"),
        "adapter must not import local runtime config module"
    );
    assert!(
        !adapter_src.contains("use crate::prompts"),
        "adapter must not import local runtime prompts module"
    );
}

#[test]
fn local_runtime_module_files_are_removed_from_adapter_crate() {
    for path in ["src/client.rs", "src/config.rs", "src/prompts.rs"] {
        assert!(
            !manifest_dir().join(path).exists(),
            "runtime module should not exist in adapter crate: {path}"
        );
    }
}
