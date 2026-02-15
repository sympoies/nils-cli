# External Tooling Dependencies

This document defines the external binaries and script-level tools used by the `nils-cli` workspace,
and provides recommended installation commands for Homebrew (macOS) and Linuxbrew (Linux).

## Scope and Intent

- Focus: runtime dependencies invoked by workspace CLIs, plus development/test tooling used by repo workflows.
- Source of truth: crate READMEs, runtime process invocations (`Command::new(...)`), and repository scripts.
- Goal: make environment setup predictable for contributors and CI-like local validation.

## 1. Runtime Dependencies (Core)

These tools are required for common command paths.

| Tool | Used By | Requirement Level | Install (brew/linuxbrew) |
|---|---|---|---|
| `git` | `git-scope`, `git-cli`, `git-summary`, `git-lock`, `semantic-commit`, `fzf-cli git-*` | Required | `brew install git` |
| `fzf` | `fzf-cli` interactive commands | Required (for `fzf-cli`) | `brew install fzf` |
| `grpcurl` | `api-grpc` unary request execution backend | Required (for `api-grpc call`/suite gRPC cases) | `brew install grpcurl` |
| `magick` (or `convert` + `identify`) | `image-processing` legacy transform subcommands (`auto-orient`, `convert`, `resize`, `rotate`, `crop`, `pad`, `flip`, `flop`, `optimize`) | Required for legacy transforms; not required for `convert --from-svg` / `svg-validate` | `brew install imagemagick` |
| `ffmpeg` | `screen-record` on Linux | Required on Linux | `brew install ffmpeg` |
| `codex` | `codex-cli agent *` flows | Required for agent commands | Install from official Codex distribution |

### 1.1 `image-processing` backend split

- `convert --from-svg` and `svg-validate`:
  - Rust-backed (`usvg`/`resvg` + Rust image encoding path).
  - Do not require ImageMagick, `convert`, or `identify`.
- Legacy transform subcommands (`auto-orient`, `convert`, `resize`, `rotate`, `crop`, `pad`,
  `flip`, `flop`, `optimize`) when not using `--from-svg`:
  - Require ImageMagick runtime (`magick`, or `convert` + `identify`).

## 2. Runtime Dependencies (Optional / Degradation Paths)

These tools enable richer behavior. Missing tools typically trigger fallback behavior or reduced UX.

| Tool | Behavior Impact | Install (brew/linuxbrew) |
|---|---|---|
| `tree` | Enables directory tree rendering in `git-scope` | `brew install tree` |
| `file` | MIME-based binary detection in `git-scope` and `git-cli commit context` | Usually preinstalled |
| `lsof` | Preferred backend for `fzf-cli port` (fallback: `netstat`) | `brew install lsof` |
| `bat` | Syntax-highlighted previews in `fzf-cli file` | `brew install bat` |
| `code` | VS Code open mode for `fzf-cli` (`--vscode`) | macOS: `brew install --cask visual-studio-code` |
| `pbcopy` / `wl-copy` / `xclip` / `xsel` | Clipboard integration for `git-cli commit context` | Linux: `brew install wl-clipboard xclip xsel` |
| `cjpeg` / `djpeg` | JPEG optimization path in `image-processing optimize` | `brew install jpeg-turbo` |
| `cwebp` / `dwebp` | WebP optimization path; macOS WebP screenshot fallback in `screen-record` | `brew install webp` |
| `pactl` | Linux audio source discovery for `screen-record --audio ...` | `brew install pulseaudio` |
| `xdg-desktop-portal` + backend + PipeWire | Wayland portal capture path (`screen-record --portal`) | Prefer distro packages |
| `open-changed-files` | Optional helper used by `fzf-cli git-commit` | Project-specific optional tool |
| `hs` (Hammerspoon CLI) | Preferred AX backend path for `macos-agent ax *` (fallback to JXA when unavailable) | `brew install --cask hammerspoon` |
| `im-select` | Required by `macos-agent input-source *` and macOS real E2E keyboard/input-source setup | `brew install im-select` |

## 2.1 Agent provider adapter maturity and runtime expectations

`agentctl` now ships with three built-in provider adapters. Runtime requirements differ by maturity.

| Provider crate | Provider ID | Maturity | Runtime requirement |
|---|---|---|---|
| `agent-provider-codex` | `codex` | `stable` | Requires `codex` binary for execute flows |
| `agent-provider-claude` | `claude` | `stub` | Compile-only stub (no external binary required yet) |
| `agent-provider-gemini` | `gemini` | `stub` | Compile-only stub (no external binary required yet) |

## 3. Development and Validation Toolchain

| Tool | Purpose | Recommended Install |
|---|---|---|
| Rust toolchain (`cargo`, `rustc`, `rustfmt`, `clippy`) | Build/lint/test pipeline | `brew install rustup-init && rustup-init -y && rustup component add rustfmt clippy` |
| `cargo-nextest` | CI-style test execution | `cargo install cargo-nextest --locked` |
| `cargo-llvm-cov` | Coverage workflows | `cargo install cargo-llvm-cov --locked` |
| `zsh` | Required for `tests/zsh/completion.test.zsh` | `brew install zsh` |
| `python3` | `scripts/workspace-bins.py` | `brew install python` |
| `bash`, `awk`, `sed` | CI helper scripts in `scripts/ci/` | Typically preinstalled |
| `bash-completion` | Bash completion loading (optional) | `brew install bash-completion` |
| `gh` | PR/release operations in GitHub-driven workflows | `brew install gh` |

## 4. Repository-Local Script Entry Points

These are repository scripts (not third-party packages):

- Install workspace binaries:
  - `./.codex/skills/nils-cli-install/scripts/nils-cli-install.sh`
- Run required repository checks:
  - `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
- Supporting utilities:
  - `scripts/workspace-bins.py`
  - `scripts/ci/coverage-summary.sh`
  - `scripts/ci/coverage-badge.sh`

## 5. `agent-docs` integration for `project-dev`

Use `agent-docs add` to register this file as a required project-level document for
`project-dev` resolution.

```bash
cargo run -p agent-docs -- add \
  --target project \
  --context project-dev \
  --scope project \
  --path BINARY_DEPENDENCIES.md \
  --required \
  --when always \
  --notes "External runtime tools required by the repo"
```

Expected stdout format:

```text
add: target=project action=<inserted|updated> config=<PROJECT_PATH>/AGENT_DOCS.toml entries=<N>
```

Verify resolution includes this document:

```bash
cargo run -p agent-docs -- resolve --context project-dev --format checklist \
  | rg "REQUIRED_DOCS_BEGIN|REQUIRED_DOCS_END|BINARY_DEPENDENCIES\\.md"
```

## 6. Recommended Install Profiles

### 6.1 Base contributor profile

```bash
brew install git gh fzf tree imagemagick webp jpeg-turbo ffmpeg bat zsh python bash-completion rustup-init im-select
```

### 6.2 Linux extra profile (audio/clipboard/network ergonomics)

```bash
brew install lsof wl-clipboard xclip xsel pulseaudio
```

## 7. Linuxbrew Bootstrap (if `brew` is not installed)

```bash
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
```

After installation, initialize Linuxbrew in shell startup (example):

```bash
eval "$(/home/linuxbrew/.linuxbrew/bin/brew shellenv)"
```

## 8. Quick Environment Verification

```bash
for c in git gh fzf tree file magick ffmpeg bat im-select; do
  if command -v "$c" >/dev/null 2>&1; then
    echo "[OK]   $c -> $(command -v "$c")"
  else
    echo "[MISS] $c"
  fi
done
```
