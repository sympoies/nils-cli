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
directory_policy() {{
  [ "$#" -eq 2 ] || return 1
  case "$1" in ''|*[!0-9]*) return 1 ;; esac
  case "$2" in ''|*[!0-7]*) return 1 ;; esac
  {{ [ "$1" = "$caller_uid" ] || [ "$1" = 0 ]; }} && [ "$((0$2 & 022))" -eq 0 ]
}}
directory_controlled() {{
  directory=${{1%/*}}
  [ -d "$directory" ] || return 1
  # GNU stat on Linux; BSD stat on macOS. Both report numeric UID and octal mode.
  metadata=$(/usr/bin/stat -c '%u %a' "$directory" 2>/dev/null) || metadata=$(/usr/bin/stat -f '%u %Lp' "$directory" 2>/dev/null) || return 1
  directory_ifs=$IFS; IFS=' '; set -- $metadata; IFS=$directory_ifs
  directory_policy "$@"
}}
caller_uid=$(/usr/bin/id -u) || {{ exit 127; }}
configured=${{AGENT_SESSION_BIN:-}}
case "$configured" in /*/{NAME}) if [ -f "$configured" ] && [ -x "$configured" ]; then selected=${{configured%/*}}/{executable}; session_real=$(resolve "$configured") && selected_real=$(resolve "$selected") && [ "${{session_real%/*}}" = "${{selected_real%/*}}" ] && [ -f "$selected_real" ] && [ -x "$selected_real" ] && directory_controlled "$selected_real" && exec "$selected_real" "$@"; exit 127; fi ;; esac
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
  case "$root" in /*) selected=$root/{executable}; if selected_real=$(resolve "$selected") && [ -f "$selected_real" ] && [ -x "$selected_real" ] && directory_controlled "$selected_real" && {{ [ -n "$configured_roots" ] || trusted_default "$selected" "$root"; }}; then IFS=$old_ifs; exec "$selected_real" "$@"; fi ;; esac
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
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::process::Command;

    #[test]
    fn directory_policy_matches_caller_root_and_foreign_owner_rules() {
        let program = selector(true);
        let functions = program.split("caller_uid=").next().unwrap();
        let script = format!("{functions}\ncaller_uid=1000\ndirectory_policy \"$1\" \"$2\"");
        for (owner, mode, accepted) in [
            ("1000", "755", true),
            ("0", "755", true),
            ("1001", "755", false),
            ("1000", "775", false),
            ("0", "757", false),
        ] {
            let status = Command::new("/bin/sh")
                .args(["-c", &script, "fixture", owner, mode])
                .status()
                .unwrap();
            assert_eq!(status.success(), accepted, "owner={owner} mode={mode}");
        }
    }

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
