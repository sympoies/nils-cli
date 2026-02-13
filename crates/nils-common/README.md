# nils-common

`nils-common` is the workspace shared helper crate for cross-CLI primitives.

Primary constraint: shared helpers must preserve behavioral parity for each consuming CLI. Moving logic into this crate must not change user-facing output text, warnings, color behavior, or exit-code contracts.

## Shared helper policy

### What belongs in `nils-common`

- Reusable helper logic used by multiple CLI crates.
- Domain-neutral primitives (process/env/shell/git/clipboard/fs internals).
- APIs that return structured results and let callers own final UX text.
- Behavior that can be covered by deterministic unit tests.

### What stays crate-local

- User-facing warning/error text (including emoji/prefix wording).
- Exit-code mapping and command-level failure policy.
- CLI-specific command composition and UX defaults.
- Product/business/domain flows that only make sense in one crate.

## Modules and purpose

- `env`: truthy parsing helpers and `NO_COLOR` presence checks.
- `shell`: POSIX single-quote escaping and ANSI stripping modes.
- `process`: command execution wrappers plus PATH lookup helpers.
- `git`: `git` command wrappers for repo probes and `rev-parse` helpers.
- `clipboard`: best-effort clipboard copy with explicit tool priority.
- `fs`: cross-platform replace/rename-overwrite helper.
- `greeting`: tiny sample helper used by `cli-template`.

## API examples

`env`:

```rust
use nils_common::env;

let starship_enabled = env::env_truthy_or("CODEX_CLI_STARSHIP", false);
let no_color = env::no_color_enabled();
println!("starship={starship_enabled}, no_color={no_color}");
```

`shell`:

```rust
use nils_common::shell::{self, AnsiStripMode, SingleQuoteEscapeStyle};

let quoted = shell::quote_posix_single_with_style("a'b", SingleQuoteEscapeStyle::Backslash);
let plain = shell::strip_ansi("\x1b[31mred\x1b[0m", AnsiStripMode::CsiSgrOnly);
assert_eq!(quoted, "'a'\\''b'");
assert_eq!(plain, "red");
```

`process`:

```rust
use nils_common::process;

assert!(process::cmd_exists("git"));
let git_path = process::find_in_path("git").expect("git should be on PATH");
let out = process::run_stdout_trimmed(git_path.to_string_lossy().as_ref(), &["--version"])
    .expect("git --version should run");
println!("{out}");
```

`git`:

```rust
use nils_common::git;

let inside = git::is_inside_work_tree().expect("git check should run");
if inside {
    let root = git::repo_root().expect("repo root check");
    println!("repo root: {root:?}");
}
```

`clipboard`:

```rust
use nils_common::clipboard::{copy_best_effort, ClipboardOutcome, ClipboardPolicy, ClipboardTool};

let tool_order = [
    ClipboardTool::Pbcopy,
    ClipboardTool::WlCopy,
    ClipboardTool::Xclip,
    ClipboardTool::Xsel,
    ClipboardTool::Clip,
];
let outcome = copy_best_effort("hello", &ClipboardPolicy::new(&tool_order));

if matches!(
    outcome,
    ClipboardOutcome::SkippedNoTool | ClipboardOutcome::SkippedFailure
) {
    eprintln!("clipboard copy unavailable; keep crate-local fallback messaging");
}
```

`fs`:

```rust
use std::path::Path;

nils_common::fs::replace_file(Path::new("tmp.out"), Path::new("final.out"))?;
# Ok::<(), std::io::Error>(())
```

## Migration conventions for parity

When introducing a shared helper at a call site:

1. Add or keep characterization tests in the caller crate first.
2. Move only primitive logic; keep a crate-local adapter for message formatting and exit-code mapping.
3. Preserve existing quote/ANSI mode choices and `NO_COLOR` behavior.
4. Keep tool/command fallback order identical (for example clipboard tool order, git probe fallback behavior).
5. Re-run crate tests that cover the touched command paths before merging.

## Non-goals

- Defining CLI-specific UX copy, warning templates, or emoji policy.
- Owning command-level business logic for a single CLI.
- Hiding meaningful behavior differences that should remain explicit in local adapters.
- Replacing specialized shared crates such as `api-testing-core`, `nils-term`, or `nils-test-support`.

## Docs

- [Docs index](docs/README.md)
