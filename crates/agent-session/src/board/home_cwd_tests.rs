//! Synthetic filesystem contracts for privacy-preserving board cwd identity.

use super::{home_relative_cwd, project_record};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::path::{Component, Path, PathBuf};

struct Fixture {
    _temp: tempfile::TempDir,
    home: PathBuf,
    alias: PathBuf,
    checkout: PathBuf,
    outside: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::TempDir::new().expect("temporary filesystem");
        let root = temp.path().canonicalize().expect("canonical fixture root");
        let home = root.join("real-home");
        let checkout = home.join("workspace/coordinator");
        let outside = root.join("outside");
        std::fs::create_dir_all(&checkout).expect("checkout");
        std::fs::create_dir_all(&outside).expect("outside directory");
        let alias = root.join("home-alias");
        std::os::unix::fs::symlink(&home, &alias).expect("home alias");
        Self {
            _temp: temp,
            home,
            alias,
            checkout,
            outside,
        }
    }

    fn project(&self, cwd: &Path, home: &Path) -> Value {
        let view = json!({
            "id": "synthetic-session",
            "agent": "codex",
            "status": "running",
            "created_at": "2030-01-01T00:00:00Z",
            "updated_at": "2030-01-01T00:00:00Z",
            "cwd": cwd.to_str().expect("fixture cwd text")
        });
        project_record(&view, "fixture", Some(home), false).expect("board record")
    }
}

// Consumer-shaped contract: only a normalized, verifiable home-relative cwd
// that resolves to the expected checkout is eligible. A null cwd stays rejected.
fn checkout_eligible(record: &Value, home: &Path, checkout: &Path) -> bool {
    let Some(projected) = record["cwd"].as_str() else {
        return false;
    };
    let relative = if projected == "~" {
        Path::new("")
    } else if let Some(relative) = projected.strip_prefix("~/") {
        Path::new(relative)
    } else {
        return false;
    };
    if relative
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
    {
        return false;
    }
    let Ok(resolved) = home.join(relative).canonicalize() else {
        return false;
    };
    let Ok(expected) = checkout.canonicalize() else {
        return false;
    };
    resolved == expected
}

#[test]
fn symlink_home_with_canonical_cwd_preserves_relative_identity() {
    let f = Fixture::new();
    assert_eq!(
        home_relative_cwd(f.checkout.to_str().unwrap(), Some(&f.alias)).as_deref(),
        Some("~/workspace/coordinator")
    );
    assert_eq!(
        home_relative_cwd(f.home.to_str().unwrap(), Some(&f.alias)).as_deref(),
        Some("~")
    );
}

#[test]
fn canonical_home_with_symlink_cwd_preserves_relative_identity() {
    let f = Fixture::new();
    let cwd = f.alias.join("workspace/coordinator");
    assert_eq!(
        home_relative_cwd(cwd.to_str().unwrap(), Some(&f.home)).as_deref(),
        Some("~/workspace/coordinator")
    );
}

#[test]
fn projected_symlink_equivalents_remain_eligible_for_a_verifiable_checkout() {
    let f = Fixture::new();
    let alias_cwd = f.alias.join("workspace/coordinator");
    for (cwd, home) in [(&f.checkout, &f.alias), (&alias_cwd, &f.home)] {
        let record = f.project(cwd, home);
        assert_eq!(record["cwd"], "~/workspace/coordinator");
        assert!(
            checkout_eligible(&record, home, &f.checkout),
            "equivalent cwd must remain eligible"
        );
        assert!(
            !checkout_eligible(&record, home, &f.outside),
            "a different checkout must remain ineligible"
        );
        assert!(
            !record["cwd"].as_str().unwrap().starts_with('/'),
            "board payload must not expose an absolute path"
        );
    }
    let outside = f.project(&f.outside, &f.alias);
    assert_eq!(outside["cwd"], Value::Null);
    assert!(!checkout_eligible(&outside, &f.alias, &f.checkout));
    assert!(!checkout_eligible(
        &json!({"cwd": null}),
        &f.alias,
        &f.checkout
    ));
}

#[test]
fn symlink_escape_outside_home_is_rejected() {
    let f = Fixture::new();
    let escape = f.home.join("escape");
    std::os::unix::fs::symlink(&f.outside, &escape).expect("outside alias");
    assert_eq!(
        home_relative_cwd(escape.to_str().unwrap(), Some(&f.home)),
        None
    );
    assert_eq!(f.project(&escape, &f.home)["cwd"], Value::Null);
}

#[test]
fn missing_cwd_fails_closed_instead_of_using_a_lexical_prefix() {
    let f = Fixture::new();
    let missing = f.home.join("missing-checkout");
    assert_eq!(
        home_relative_cwd(missing.to_str().unwrap(), Some(&f.home)),
        None
    );
    assert_eq!(f.project(&missing, &f.home)["cwd"], Value::Null);
}

#[test]
fn unusable_home_fails_closed_instead_of_using_a_lexical_prefix() {
    let f = Fixture::new();
    let missing = f.home.join("missing-home");
    let missing_cwd = missing.join("workspace");
    assert_eq!(
        home_relative_cwd(missing_cwd.to_str().unwrap(), Some(&missing)),
        None
    );
    let broken = f.home.join("broken-home");
    std::os::unix::fs::symlink(&missing, &broken).expect("broken alias");
    assert_eq!(
        home_relative_cwd(broken.join("workspace").to_str().unwrap(), Some(&broken)),
        None
    );
}

#[test]
fn a_home_alias_resolving_to_root_is_rejected() {
    let f = Fixture::new();
    let root_alias = f.home.join("root-alias");
    std::os::unix::fs::symlink(Path::new("/"), &root_alias).expect("root alias");
    let cwd = root_alias.join(f.checkout.strip_prefix("/").expect("absolute checkout"));
    assert_eq!(
        home_relative_cwd(cwd.to_str().unwrap(), Some(&root_alias)),
        None
    );
}

#[test]
fn ordinary_existing_paths_and_identity_rejections_are_preserved() {
    let f = Fixture::new();
    assert_eq!(
        home_relative_cwd(f.checkout.to_str().unwrap(), Some(&f.home)).as_deref(),
        Some("~/workspace/coordinator")
    );
    assert_eq!(
        home_relative_cwd(f.home.to_str().unwrap(), Some(&f.home)).as_deref(),
        Some("~")
    );
    assert_eq!(
        home_relative_cwd(f.outside.to_str().unwrap(), Some(&f.home)),
        None
    );
    assert_eq!(home_relative_cwd("relative/workspace", Some(&f.home)), None);
    assert_eq!(
        home_relative_cwd(f.home.join("../outside").to_str().unwrap(), Some(&f.home)),
        None
    );
    assert_eq!(home_relative_cwd(f.checkout.to_str().unwrap(), None), None);
    assert_eq!(
        home_relative_cwd(f.checkout.to_str().unwrap(), Some(Path::new("/"))),
        None
    );
}

#[cfg(target_os = "macos")]
#[test]
fn native_macos_home_aliases_preserve_relative_identity_when_available() {
    let Ok(canonical_var) = Path::new("/var").canonicalize() else {
        return;
    };
    if canonical_var != Path::new("/private/var") {
        return;
    }
    let temp = tempfile::TempDir::new_in("/var/tmp").expect("native alias fixture");
    let home = temp.path().join("home");
    let checkout = home.join("workspace/coordinator");
    std::fs::create_dir_all(&checkout).expect("native alias checkout");
    let canonical_home = home.canonicalize().expect("canonical home");
    let canonical_cwd = checkout.canonicalize().expect("canonical cwd");
    for (cwd, home) in [(&canonical_cwd, &home), (&checkout, &canonical_home)] {
        assert_eq!(
            home_relative_cwd(cwd.to_str().unwrap(), Some(home)).as_deref(),
            Some("~/workspace/coordinator")
        );
    }
}
