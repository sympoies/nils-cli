//! Runtime-selected provider entrypoints.

use crate::session_binary::NAME;

/// Shell program used as an argv entrypoint; the positional arguments are
/// forwarded unchanged to the selected executable.
pub fn selector(hook: bool) -> String {
    let executable = if hook { "agent-hook" } else { NAME };
    format!(
        r#"resolve() {{
  target=$1; count=0
  directory=$(CDPATH= cd -P "${{target%/*}}" 2>/dev/null && pwd -P) || return 1
  target=$directory/${{target##*/}}
  while [ -L "$target" ]; do
    count=$((count + 1)); [ "$count" -le 40 ] || return 1
    link=$(/usr/bin/readlink "$target") || return 1
    case "$link" in /*) target=$link ;; *) target=${{target%/*}}/$link ;; esac
    directory=$(CDPATH= cd -P "${{target%/*}}" 2>/dev/null && pwd -P) || return 1
    target=$directory/${{target##*/}}
  done
  printf '%s' "$target"
}}
configured=${{AGENT_SESSION_BIN:-}}
case "$configured" in /*/{NAME}) if [ -f "$configured" ] && [ -x "$configured" ]; then selected=${{configured%/*}}/{executable}; session_real=$(resolve "$configured") && selected_real=$(resolve "$selected") && [ "${{session_real%/*}}" = "${{selected_real%/*}}" ] && [ -f "$selected" ] && [ -x "$selected" ] && exec "$selected" "$@"; exit 127; fi ;; esac
configured_roots=${{AGENT_RUNTIME_TRUSTED_CLI_ROOT:-}}
roots=${{configured_roots:-/opt/homebrew/bin:/home/linuxbrew/.linuxbrew/bin:/usr/local/bin:/usr/bin:${{HOME:-/nonexistent}}/.local/nils-cli/bin}}
trusted_default() {{
  target=$1; approved=$2
  approved=$(CDPATH= cd -P "$approved" 2>/dev/null && pwd -P) || return 1
  target=$(resolve "$target") || return 1
  case "$approved" in
    /opt/homebrew/bin|/home/linuxbrew/.linuxbrew/bin)
      case "$target" in "$approved/{executable}"|"${{approved%/bin}}/Cellar/nils-cli/"*) return 0 ;; esac ;;
    *) [ "$target" = "$approved/{executable}" ] && return 0 ;;
  esac
  return 1
}}
set -f
old_ifs=$IFS; IFS=:
for root in $roots; do
  case "$root" in /*) selected=$root/{executable}; if [ -f "$selected" ] && [ -x "$selected" ] && {{ [ -n "$configured_roots" ] || trusted_default "$selected" "$root"; }}; then IFS=$old_ifs; exec "$selected" "$@"; fi ;; esac
done
exit 127"#
    )
}

pub fn argv(hook: bool, args: &[&str]) -> Vec<String> {
    let mut result = vec![
        "/bin/sh".into(),
        "-c".into(),
        selector(hook),
        "provider-entrypoint".into(),
    ];
    result.extend(args.iter().map(|arg| (*arg).to_string()));
    result
}

pub fn command(hook: bool, args: &[&str]) -> String {
    argv(hook, args)
        .iter()
        .map(|arg| format!("'{}'", arg.replace('\'', "'\\''")))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::{argv, selector};
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::process::Command;

    #[test]
    fn default_root_rejects_a_link_outside_the_approved_directory() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        fs::create_dir_all(&root).unwrap();
        let candidate = root.join("agent-hook");
        fs::write(&candidate, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&candidate, fs::Permissions::from_mode(0o755)).unwrap();
        let program = selector(true);
        let resolve = program
            .split("configured=${AGENT_SESSION_BIN:-}")
            .next()
            .unwrap();
        let function = program
            .split("trusted_default() {")
            .nth(1)
            .unwrap()
            .split("\nset -f")
            .next()
            .unwrap();
        let script =
            format!("{resolve}\ntrusted_default() {{{function}\ntrusted_default \"$1\" \"$2\"");
        let check = || {
            Command::new("/bin/sh")
                .args(["-c", &script, "fixture"])
                .arg(&candidate)
                .arg(&root)
                .status()
                .unwrap()
                .success()
        };
        assert!(check());
        let outside = temp.path().join("outside");
        fs::rename(&candidate, &outside).unwrap();
        symlink(&outside, &candidate).unwrap();
        assert!(!check());
    }

    #[test]
    fn configured_session_refuses_a_companion_linked_to_another_directory() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("selected");
        fs::create_dir_all(&root).unwrap();
        let session = root.join(crate::session_binary::NAME);
        let other = temp.path().join("other-hook");
        for path in [&session, &other] {
            fs::write(path, "#!/bin/sh\nexit 0\n").unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        symlink(other, root.join("agent-hook")).unwrap();
        let args = argv(true, &["dispatch"]);
        let status = Command::new(&args[0])
            .args(&args[1..])
            .env("AGENT_SESSION_BIN", session)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(127));
    }
}
