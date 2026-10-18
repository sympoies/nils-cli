//! Composer observations for fixtures whose delivery uses the native channel.
use super::*;

pub(super) fn empty_composer_tmux(dir: &Path) -> PathBuf {
    let bin = minimal_tmux(dir);
    let script = fs::read_to_string(&bin).unwrap().replace(
        "capture-pane) printf 'pane\\n'; exit 0 ;;",
        "capture-pane) printf '────────────────────\\n❯ \\n────────────────────\\n› \\n'; exit 0 ;;",
    );
    fs::write(&bin, script).unwrap();
    bin
}

/// Show an empty composer before Enter and the provider's queued pane after it.
pub(super) fn minimal_tmux_showing(dir: &Path, pane: &str) -> PathBuf {
    let bin = minimal_tmux(dir);
    let shown = dir.join("shown-pane");
    let entered = dir.join("entered");
    fs::write(&shown, pane).unwrap();
    let script = fs::read_to_string(&bin).unwrap().replace(
        "capture-pane) printf 'pane\\n'; exit 0 ;;",
        &format!("capture-pane) if [ -f '{entered}' ]; then cat '{shown}'; else printf '────────────────────\\n❯ \\n────────────────────\\n'; fi; exit 0 ;;\n  send-keys) touch '{entered}'; exit 0 ;;",
            entered=entered.display(), shown=shown.display()),
    );
    fs::write(&bin, script).unwrap();
    bin
}
