use pretty_assertions::assert_eq;
use std::collections::BTreeSet;
use std::process::Command;

fn groups(inventory: &str) -> BTreeSet<u32> {
    inventory
        .split_whitespace()
        .map(|group| group.parse().expect("numeric group inventory"))
        .collect()
}

pub(super) fn manager_groups() -> String {
    // Independent of agent-hook's runner properties and shell environment.
    let mut command = Command::new("/usr/bin/systemd-run");
    if let Ok(slice) = std::env::var("NILS_CLI_GATE_SLICE") {
        command.arg(format!("--slice={slice}"));
    }
    let output = command
        .args([
            "--user",
            "--quiet",
            "--wait",
            "--pipe",
            "--collect",
            "--property=MemoryMax=6G",
            "--property=MemorySwapMax=0",
            "--",
            "/usr/bin/id",
            "-G",
        ])
        .output()
        .expect("user-manager group inventory");
    assert!(
        output.status.success(),
        "user-manager group inventory failed"
    );
    String::from_utf8(output.stdout).expect("manager groups are utf8")
}

pub(super) fn assert_preserved_groups(caller: &str, manager: &str, observed: &str) {
    let caller = groups(caller);
    let manager = groups(manager);
    assert!(!manager.is_empty(), "empty user-manager group inventory");
    if caller != manager {
        eprintln!(
            "finish-line group context: login/caller and user-manager inventories differ \
             (caller={}, manager={}); validation inherits user-manager groups, \
             not login-process groups",
            caller.len(),
            manager.len(),
        );
    }
    assert_eq!(
        groups(observed),
        manager,
        "danger-full-access changed the user-manager supplementary groups",
    );
}

#[test]
fn manager_groups_are_preserved_when_login_inventory_differs() {
    // A login can inherit an extra group absent from the existing user manager.
    assert_preserved_groups("1000 2000 3000", "1000 2000", "2000 1000");
}

#[test]
fn manager_only_groups_are_part_of_the_preservation_contract() {
    assert_preserved_groups("1000", "1000 2000", "2000 1000");
}

#[test]
#[should_panic(expected = "changed the user-manager supplementary groups")]
fn losing_a_manager_group_still_fails() {
    assert_preserved_groups("1000 2000 3000", "1000 2000", "1000");
}

#[test]
#[should_panic(expected = "changed the user-manager supplementary groups")]
fn adding_a_group_still_fails() {
    assert_preserved_groups("1000 2000 3000", "1000 2000", "1000 2000 3000");
}
