use pretty_assertions::assert_eq;
use tempfile::TempDir;

use crate::common::MacosAgentHarness;

#[test]
fn doctor_missing_backend_reports_install_guidance_without_creating_storage() {
    for existing_root in [false, true] {
        let harness = MacosAgentHarness::new();
        let cwd = TempDir::new().expect("fixture");
        let root = cwd.path().join("missing/backend");
        if existing_root {
            std::fs::create_dir_all(&root).expect("empty backend storage");
        }
        for strict in [false, true] {
            let mut args = vec!["--format", "json", "doctor"];
            if strict {
                args.push("--strict");
            }
            let options = harness.cmd_options(cwd.path()).with_env(
                "NILS_MACOS_AGENT_BACKEND_ROOT",
                root.to_str().expect("root"),
            );
            let output = harness.run_with_options(cwd.path(), &args, options);
            assert_eq!(
                output.code,
                if strict { 77 } else { 0 },
                "{}",
                output.stderr_text()
            );
            let report = output.stdout_json();
            let report = &report["result"];
            assert_eq!(report["ready"], false);
            assert_eq!(report["backend"]["ready"], false);
            let checks = report["backend"]["checks"]
                .as_array()
                .expect("backend checks");
            assert!(checks.iter().any(|check| {
                check["id"] == "backend"
                    && check["status"] == "fail"
                    && check["message"].as_str().is_some_and(|message| {
                        message.contains("not installed")
                            && message.contains("macos-agent backend install")
                    })
            }));
            for probe in ["runtime", "permissions", "bridge"] {
                assert_eq!(report[probe]["status"], "blocked");
            }
            assert_eq!(root.exists(), existing_root);
            if existing_root {
                assert_eq!(std::fs::read_dir(&root).expect("storage").count(), 1);
                assert!(root.join(".backend-lifecycle.lock").is_file());
            } else {
                assert!(!cwd.path().join("missing").exists());
            }
        }
    }
}
