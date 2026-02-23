pub const STUB_LOG_ENV: &str = "NILS_TEST_STUB_LOG";

fn log_prefix(tool: &str) -> String {
    format!(
        "if [[ -n \"${{{STUB_LOG_ENV}:-}}\" ]]; then echo \"{tool} $*\" >> \"${{{STUB_LOG_ENV}}}\"; fi\n"
    )
}

pub fn identify_stub_script() -> String {
    let mut script = String::from("#!/bin/bash\nset -euo pipefail\n");
    script.push_str(&log_prefix("identify"));
    script.push_str(
        r#"
path="${@: -1}"
name="$(/usr/bin/basename "$path")"
ext="${name##*.}"
ext="$(/usr/bin/tr '[:upper:]' '[:lower:]' <<<"$ext")"

fmt="PNG"
if [[ "$ext" == "jpg" || "$ext" == "jpeg" ]]; then
  fmt="JPEG"
elif [[ "$ext" == "webp" ]]; then
  fmt="WEBP"
fi

channels="rgb"
if [[ "$name" == *alpha* ]]; then
  channels="rgba"
fi

echo "${fmt}|100|50|${channels}|1"
"#,
    );
    script
}

pub fn convert_stub_script() -> String {
    let mut script = String::from("#!/bin/bash\nset -euo pipefail\n");
    script.push_str(&log_prefix("convert"));
    script.push_str(
        r#"
in="$1"
out="${@: -1}"

dir="$(/usr/bin/dirname "$out")"
/bin/mkdir -p "$dir"
/bin/cp "$in" "$out"
"#,
    );
    script
}

pub fn magick_stub_script() -> String {
    let mut script = String::from("#!/bin/bash\nset -euo pipefail\n");
    script.push_str(&log_prefix("magick"));
    script.push_str(
        r#"
if [[ "${1:-}" == "identify" ]]; then
  shift
  path="${@: -1}"
  name="$(/usr/bin/basename "$path")"
  ext="${name##*.}"
  ext="$(/usr/bin/tr '[:upper:]' '[:lower:]' <<<"$ext")"

  fmt="PNG"
  if [[ "$ext" == "jpg" || "$ext" == "jpeg" ]]; then
    fmt="JPEG"
  elif [[ "$ext" == "webp" ]]; then
    fmt="WEBP"
  fi

  channels="rgb"
  if [[ "$name" == *alpha* ]]; then
    channels="rgba"
  fi

  echo "${fmt}|100|50|${channels}|1"
  exit 0
fi

in="$1"
out="${@: -1}"
dir="$(/usr/bin/dirname "$out")"
/bin/mkdir -p "$dir"
/bin/cp "$in" "$out"
"#,
    );
    script
}

pub fn dwebp_stub_script() -> String {
    let mut script = String::from("#!/bin/bash\nset -euo pipefail\n");
    script.push_str(&log_prefix("dwebp"));
    script.push_str(
        r#"
in="$1"
out=""
prev=""
for a in "$@"; do
  if [[ "$prev" == "-o" ]]; then
    out="$a"
    break
  fi
  prev="$a"
done

if [[ -z "$out" ]]; then
  echo "dwebp: missing -o" >&2
  exit 1
fi

dir="$(/usr/bin/dirname "$out")"
/bin/mkdir -p "$dir"
/bin/cp "$in" "$out"
"#,
    );
    script
}

pub fn cwebp_stub_script() -> String {
    let mut script = String::from("#!/bin/bash\nset -euo pipefail\n");
    script.push_str(&log_prefix("cwebp"));
    script.push_str(
        r#"
out=""
src=""
prev=""
out_next=0
for a in "$@"; do
  if [[ "$out_next" == "1" ]]; then
    out="$a"
    src="$prev"
    break
  fi
  if [[ "$a" == "-o" ]]; then
    out_next=1
  else
    prev="$a"
  fi
done

if [[ -z "$out" ]]; then
  echo "cwebp: missing -o" >&2
  exit 1
fi

dir="$(/usr/bin/dirname "$out")"
/bin/mkdir -p "$dir"
/bin/cp "$src" "$out"
"#,
    );
    script
}

pub fn djpeg_stub_script() -> String {
    let mut script = String::from("#!/bin/bash\nset -euo pipefail\n");
    script.push_str(&log_prefix("djpeg"));
    script.push_str(
        r#"
in="$1"
/bin/cat "$in"
"#,
    );
    script
}

pub fn cjpeg_stub_script() -> String {
    let mut script = String::from("#!/bin/bash\nset -euo pipefail\n");
    script.push_str(&log_prefix("cjpeg"));
    script.push_str(
        r#"
out=""
prev=""
for a in "$@"; do
  if [[ "$prev" == "-outfile" ]]; then
    out="$a"
    break
  fi
  prev="$a"
done

if [[ -z "$out" ]]; then
  echo "cjpeg: missing -outfile" >&2
  exit 1
fi

dir="$(/usr/bin/dirname "$out")"
/bin/mkdir -p "$dir"
/bin/cat > "$out"
"#,
    );
    script
}

pub fn fzf_stub_script() -> &'static str {
    r#"#!/bin/bash
set -euo pipefail

if [[ -n "${NILS_TEST_STUB_LOG:-}" ]]; then
  echo "fzf $*" >> "${NILS_TEST_STUB_LOG}"
fi

dir="${FZF_STUB_OUT_DIR:?FZF_STUB_OUT_DIR is required}"
counter="$dir/.counter"
n=1
if [[ -f "$counter" ]]; then
  n=$(( $(/bin/cat "$counter") + 1 ))
fi
echo "$n" > "$counter"

out="$dir/$n.out"
code_file="$dir/$n.code"
if [[ -f "$out" ]]; then
  /bin/cat "$out"
fi

if [[ -f "$code_file" ]]; then
  exit "$(/bin/cat "$code_file")"
fi
exit 0
"#
}

pub fn bat_stub_script() -> &'static str {
    r#"#!/bin/bash
set -euo pipefail
if [[ -n "${NILS_TEST_STUB_LOG:-}" ]]; then
  echo "bat $*" >> "${NILS_TEST_STUB_LOG}"
fi
file="${@: -1}"
if [[ -f "$file" ]]; then
  /bin/cat "$file"
fi
"#
}

pub fn tree_stub_script() -> &'static str {
    r#"#!/bin/bash
set -euo pipefail
if [[ -n "${NILS_TEST_STUB_LOG:-}" ]]; then
  echo "tree $*" >> "${NILS_TEST_STUB_LOG}"
fi
echo "."
"#
}

pub fn file_stub_script() -> &'static str {
    r#"#!/bin/bash
set -euo pipefail
if [[ -n "${NILS_TEST_STUB_LOG:-}" ]]; then
  echo "file $*" >> "${NILS_TEST_STUB_LOG}"
fi
echo "text/plain"
"#
}

fn clipboard_copy_stub_script(tool: &str) -> String {
    let mut script = String::from("#!/bin/bash\nset -euo pipefail\n");
    script.push_str(&log_prefix(tool));
    script.push_str(
        r#"
if [[ ! -t 0 ]]; then
  /bin/cat >/dev/null
fi
exit 0
"#,
    );
    script
}

pub fn pbcopy_stub_script() -> String {
    clipboard_copy_stub_script("pbcopy")
}

pub fn wl_copy_stub_script() -> String {
    clipboard_copy_stub_script("wl-copy")
}

pub fn xclip_stub_script() -> String {
    clipboard_copy_stub_script("xclip")
}

pub fn xsel_stub_script() -> String {
    clipboard_copy_stub_script("xsel")
}

pub fn git_scope_stub_script() -> String {
    let mut script = String::from("#!/bin/bash\nset -euo pipefail\n");
    script.push_str(&log_prefix("git-scope"));
    script.push_str("\nexit 0\n");
    script
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_all_contains(haystack: &str, needles: &[&str]) {
        for needle in needles {
            assert!(
                haystack.contains(needle),
                "expected script to contain `{needle}`\nscript:\n{haystack}"
            );
        }
    }

    #[test]
    fn identify_stub_script_logs_and_detects_common_formats() {
        let script = identify_stub_script();
        assert_all_contains(
            &script,
            &[
                "#!/bin/bash",
                "set -euo pipefail",
                "echo \"identify $*\"",
                "basename",
                "fmt=\"PNG\"",
                "fmt=\"JPEG\"",
                "fmt=\"WEBP\"",
                "channels=\"rgba\"",
                "echo \"${fmt}|100|50|${channels}|1\"",
            ],
        );
    }

    #[test]
    fn convert_and_magick_stub_scripts_copy_input_to_output() {
        let convert = convert_stub_script();
        assert_all_contains(
            &convert,
            &[
                "echo \"convert $*\"",
                "in=\"$1\"",
                "out=\"${@: -1}\"",
                "/bin/mkdir -p \"$dir\"",
                "/bin/cp \"$in\" \"$out\"",
            ],
        );

        let magick = magick_stub_script();
        assert_all_contains(
            &magick,
            &[
                "echo \"magick $*\"",
                "if [[ \"${1:-}\" == \"identify\" ]]; then",
                "fmt=\"JPEG\"",
                "fmt=\"WEBP\"",
                "channels=\"rgba\"",
                "exit 0",
                "in=\"$1\"",
                "out=\"${@: -1}\"",
                "/bin/cp \"$in\" \"$out\"",
            ],
        );
    }

    #[test]
    fn webp_and_jpeg_stub_scripts_cover_required_output_flags() {
        let dwebp = dwebp_stub_script();
        assert_all_contains(
            &dwebp,
            &[
                "echo \"dwebp $*\"",
                "if [[ \"$prev\" == \"-o\" ]]; then",
                "dwebp: missing -o",
                "/bin/cp \"$in\" \"$out\"",
            ],
        );

        let cwebp = cwebp_stub_script();
        assert_all_contains(
            &cwebp,
            &[
                "echo \"cwebp $*\"",
                "if [[ \"$a\" == \"-o\" ]]; then",
                "cwebp: missing -o",
                "src=\"$prev\"",
                "/bin/cp \"$src\" \"$out\"",
            ],
        );

        let djpeg = djpeg_stub_script();
        assert_all_contains(
            &djpeg,
            &["echo \"djpeg $*\"", "in=\"$1\"", "/bin/cat \"$in\""],
        );

        let cjpeg = cjpeg_stub_script();
        assert_all_contains(
            &cjpeg,
            &[
                "echo \"cjpeg $*\"",
                "if [[ \"$prev\" == \"-outfile\" ]]; then",
                "cjpeg: missing -outfile",
                "/bin/cat > \"$out\"",
            ],
        );
    }

    #[test]
    fn generic_cli_stub_scripts_emit_expected_contracts() {
        let fzf = fzf_stub_script();
        assert_all_contains(
            fzf,
            &[
                "echo \"fzf $*\"",
                "FZF_STUB_OUT_DIR",
                "counter=\"$dir/.counter\"",
                "out=\"$dir/$n.out\"",
                "code_file=\"$dir/$n.code\"",
            ],
        );

        let bat = bat_stub_script();
        assert_all_contains(
            bat,
            &["echo \"bat $*\"", "file=\"${@: -1}\"", "/bin/cat \"$file\""],
        );

        let tree = tree_stub_script();
        assert_all_contains(tree, &["echo \"tree $*\"", "echo \".\""]);

        let file = file_stub_script();
        assert_all_contains(file, &["echo \"file $*\"", "echo \"text/plain\""]);
    }

    #[test]
    fn clipboard_and_git_scope_stubs_share_logging_prefix() {
        for script in [
            pbcopy_stub_script(),
            wl_copy_stub_script(),
            xclip_stub_script(),
            xsel_stub_script(),
        ] {
            assert_all_contains(
                &script,
                &[
                    "#!/bin/bash",
                    "set -euo pipefail",
                    "if [[ -n \"${NILS_TEST_STUB_LOG:-}\" ]]",
                    "if [[ ! -t 0 ]]; then",
                    "/bin/cat >/dev/null",
                    "exit 0",
                ],
            );
        }

        let git_scope = git_scope_stub_script();
        assert_all_contains(&git_scope, &["echo \"git-scope $*\"", "exit 0"]);
    }
}
