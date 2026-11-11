//! An exhausted composer observation budget is distinct from changed identity.
use super::{Fixture, fs};
use pretty_assertions::assert_eq;

#[test]
fn retry_identity_preserves_deadline_and_mismatch_refusals() {
    for (mode, expected) in [
        ("stall", "send-guard-observation-timeout"),
        ("unavailable", "send-identity-unobservable"),
        ("replace", "send-identity-mismatch"),
    ] {
        let fixture = Fixture::new("codex");
        fs::write(fixture.root().join("identity-mode"), mode).unwrap();
        fs::rename(
            fixture.root().join("tmux"),
            fixture.root().join("tmux-original"),
        )
        .unwrap();
        fs::write(
            fixture.root().join("tmux"),
            r#"#!/bin/sh
root=${0%/*}
if [ "$1" = capture-pane ] && [ -e "$root/paste-completed" ]; then
    # The submission receipt takes an unstyled snapshot before guard admission.
    # Arm only on the styled composer observation inside the submit guard.
    for arg in "$@"; do
        if [ "$arg" = -e ]; then touch "$root/retry-pending"; fi
    done
    exit 1
fi
if [ "$1" = show-environment ] && [ -e "$root/retry-pending" ] &&
    [ ! -e "$root/identity-attempted" ]; then
    touch "$root/identity-attempted"
    read -r mode <"$root/identity-mode"
    if [ "$mode" = stall ]; then
        # The retry has only 500 ms. Its command timeout must kill this process
        # group and report budget exhaustion, without retrying or delivering.
        exec sleep 10
    fi
    if [ "$mode" = unavailable ]; then exit 1; fi
    printf '%s=replaced\n' "$4"
    exit 0
fi
exec "$root/tmux-original" "$@"
"#,
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            fixture.root().join("tmux"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        let output = fixture.send(&["--require-empty-composer"]);
        assert!(fixture.root().join("identity-attempted").exists());
        fixture.refused(&output, expected);
        let details = &output.stdout_json()["error"]["details"];
        assert_eq!(details["stage"], "submit");
        assert_eq!(details["pasted"], true);
        assert_eq!(details["submitted"], false);
        assert_eq!(
            fs::read_to_string(fixture.root().join("composer")).unwrap(),
            "continue-fixture"
        );
        assert!(fixture.calls().iter().all(|call| {
            call[0] != "send-keys"
                || !matches!(call.last().unwrap().as_str(), "Enter" | "BSpace" | "DC")
        }));
        assert!(!fixture.root().join("submissions").exists());
    }
}
