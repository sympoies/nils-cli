#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub struct CmdOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub fn image_processing_bin() -> PathBuf {
    if let Ok(bin) = std::env::var("CARGO_BIN_EXE_image-processing")
        .or_else(|_| std::env::var("CARGO_BIN_EXE_image_processing"))
    {
        return PathBuf::from(bin);
    }

    let exe = std::env::current_exe().expect("current exe");
    let target_dir = exe.parent().and_then(|p| p.parent()).expect("target dir");
    let bin = target_dir.join("image-processing");
    if bin.exists() {
        return bin;
    }

    panic!("image-processing binary path: NotPresent");
}

pub fn run_image_processing(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> CmdOutput {
    let mut cmd = Command::new(image_processing_bin());
    cmd.args(args)
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());

    for (k, v) in envs {
        cmd.env(k, v);
    }

    let output = cmd.output().expect("run image-processing");
    CmdOutput {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

pub fn make_stub_dir() -> tempfile::TempDir {
    tempfile::TempDir::new().expect("tempdir")
}

pub fn write_exe(dir: &Path, name: &str, content: &str) {
    let path = dir.join(name);
    fs::write(&path, content).expect("write stub");
    let mut perms = fs::metadata(&path).expect("meta").permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o755);
    }
    fs::set_permissions(&path, perms).expect("chmod stub");
}

pub fn identify_stub_script() -> &'static str {
    r#"#!/bin/bash
set -euo pipefail

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
"#
}

pub fn convert_stub_script() -> &'static str {
    r#"#!/bin/bash
set -euo pipefail

in="$1"
out="${@: -1}"

dir="$(/usr/bin/dirname "$out")"
/bin/mkdir -p "$dir"
/bin/cp "$in" "$out"
"#
}

pub fn magick_stub_script() -> &'static str {
    r#"#!/bin/bash
set -euo pipefail

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
"#
}

pub fn dwebp_stub_script() -> &'static str {
    r#"#!/bin/bash
set -euo pipefail

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
"#
}

pub fn cwebp_stub_script() -> &'static str {
    r#"#!/bin/bash
set -euo pipefail

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
"#
}

pub fn djpeg_stub_script() -> &'static str {
    r#"#!/bin/bash
set -euo pipefail

in="$1"
/bin/cat "$in"
"#
}

pub fn cjpeg_stub_script() -> &'static str {
    r#"#!/bin/bash
set -euo pipefail

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
"#
}
