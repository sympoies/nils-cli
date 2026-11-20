//! Synchronous publication replies for the replacement quarantine fixture.
use serde_json::Value;
use std::cell::RefCell;
use std::path::{Path, PathBuf};

thread_local! {
    static ROOT: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

pub(crate) struct Guard;

pub(crate) fn install(root: &Path) -> Guard {
    ROOT.with_borrow_mut(|current| {
        assert!(current.is_none());
        *current = Some(root.to_path_buf());
    });
    Guard
}

impl Drop for Guard {
    fn drop(&mut self) {
        ROOT.with_borrow_mut(|current| *current = None);
    }
}

pub(super) fn response(argv: &[String], args: &[&str]) -> Option<Value> {
    ROOT.with_borrow(|root| {
        let root = root.as_ref()?;
        let [broker] = argv else {
            return None;
        };
        if Path::new(broker) != root.join("broker") {
            return None;
        }
        let file = match args {
            ["list", "--provider", "claude", "--format", "json"] => "inventory.json",
            [
                "credential-state",
                "--provider",
                "claude",
                "--account",
                _,
                "--format",
                "json",
            ] => "credential-state.json",
            _ => return None,
        };
        let bytes = std::fs::read(root.join(file)).expect("read committed publication fixture");
        Some(serde_json::from_slice(&bytes).expect("decode publication fixture response"))
    })
}
