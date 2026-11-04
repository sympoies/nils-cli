//! Preserve the journal while exposing the required empty Claude composer.
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(in crate::serve::tests) fn tmux(dir: &Path) -> (PathBuf, PathBuf) {
    let (bin, journal) = super::super::recording_tmux(dir);
    let script = fs::read_to_string(&bin).unwrap().replacen(
        "capture-pane) printf 'pane\\n'; exit 0 ;;",
        "capture-pane) printf '────────────────────\\n❯ \\n────────────────────\\n'; exit 0 ;;",
        1,
    );
    fs::write(&bin, script).unwrap();
    (bin, journal)
}
