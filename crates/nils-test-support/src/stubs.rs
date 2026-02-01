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
