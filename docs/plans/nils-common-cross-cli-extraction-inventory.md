# nils-common Cross-CLI Extraction Inventory

## Scope and parity guardrails
- Scope: Task 1.1 inventory for cross-CLI helper extraction into `crates/nils-common`.
- Non-goal: behavior changes. Output text, warning copy, color behavior, and exit codes must stay byte-for-byte compatible unless a later task explicitly approves changes.
- Decision labels:
  - `extract`: move helper directly into `nils-common` with no per-CLI behavior fork.
  - `adapt`: extract a primitive, keep thin per-CLI adapters for parity-sensitive behavior.
  - `keep local`: do not extract in this refactor stage.

## Scoring rubric
- Reuse breadth:
  - `High`: used across >=4 crates or >=10 call sites.
  - `Medium`: used across 2-3 crates or 4-9 call sites.
  - `Low`: single crate or <=3 call sites.
- Behavioral risk:
  - `High`: user-visible output/warning/exit-code drift likely if unified naively.
  - `Medium`: semantic differences exist but can be adapter-preserved.
  - `Low`: pure/internal helper with no output coupling.
- Migration effort:
  - `S`: small signature-only move.
  - `M`: moderate adapter + test updates.
  - `L`: wide migration with multiple parity branches.

## Inventory (priority targets + known duplicate call sites)
| ID | Helper cluster | Candidate `nils-common` module | Known duplicate call sites | Reuse breadth | Behavioral risk | Migration effort | Decision | Rationale |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| P-1 | Command lookup (`cmd_exists` / PATH scan) | `process` | Defs: `crates/fzf-cli/src/util.rs:40`, `crates/git-cli/src/util.rs:5`, `crates/codex-cli/src/agent/commit.rs:315`; key calls: `crates/fzf-cli/src/open.rs:74`, `crates/fzf-cli/src/defs/block_preview.rs:122`, `crates/git-cli/src/commit.rs:63`, `crates/git-cli/src/clipboard.rs:13` | High | Medium | S | adapt | `nils-common::process::find_in_path` already exists; keep adapters where slash-path/executable-bit behavior must stay unchanged. |
| P-2 | Generic process capture (`run_output` / `run_capture`) | `process` | Defs: `crates/fzf-cli/src/util.rs:65`, `crates/fzf-cli/src/util.rs:84`, `crates/git-cli/src/util.rs:13`, `crates/git-cli/src/util.rs:22`; key calls: `crates/fzf-cli/src/git_branch.rs:10`, `crates/fzf-cli/src/git_checkout.rs:47`, `crates/git-cli/src/utils.rs:149` | High | High | M | adapt | Error strings currently differ in stderr/stdout concatenation and context wording (`spawn {cmd}` vs other forms); preserve by adapter wrappers. |
| E-1 | Truthy env parsing (`1/true/yes/on` family) | `env` | Defs/calls: `crates/codex-cli/src/starship/mod.rs:90`, `crates/codex-cli/src/starship/mod.rs:97`, `crates/codex-cli/src/rate_limits/mod.rs:99`, `crates/screen-record/src/test_mode.rs:7`, `crates/screen-record/src/linux/portal.rs:273`, `crates/git-scope/src/main.rs:206`, `crates/fzf-cli/src/util.rs:22` | High | Medium | S | extract | Core parser semantics overlap strongly; keep fzf's extra `"y"` acceptance via tiny local wrapper if needed. |
| E-2 | `NO_COLOR` gating / color enable baseline | `env` | `crates/codex-cli/src/rate_limits/ansi.rs:5`, `crates/codex-cli/src/starship/render.rs:98`, `crates/git-scope/src/main.rs:132`, `crates/git-lock/src/diff.rs:74`, `crates/git-cli/src/commit.rs:209` | High | Medium | S | adapt | Shared `no_color_enabled()` is extractable, but CLI-specific color fallbacks (TTY checks, starship env overrides) must remain local. |
| S-1 | Shell single-quote escaping | `shell` | Defs/calls: `crates/codex-cli/src/config.rs:81`, `crates/fzf-cli/src/util.rs:111`, `crates/git-cli/src/utils.rs:180`, `crates/image-processing/src/util.rs:191`, call site `crates/fzf-cli/src/directory.rs:110` | High | High | M | adapt | Two literal escape styles are in use (`'\\''` vs `'"'"'`) and image-processing has safe-char passthrough; adapters required for output parity. |
| S-2 | ANSI stripping | `shell` | Defs/calls: `crates/fzf-cli/src/util.rs:93`, `crates/git-cli/src/commit.rs:225`, `crates/git-scope/src/render.rs:251`, call sites `crates/fzf-cli/src/git_commit_select.rs:64`, `crates/git-cli/src/commit.rs:221`, `crates/git-scope/src/render.rs:216` | Medium | High | S | adapt | Current parsers are not equivalent (fzf strips broader CSI terminators; others target SGR `m`); expose explicit variants to avoid drift. |
| G-1 | Git repo probe (`rev-parse` / inside-work-tree checks) | `git` | Defs/calls: `crates/fzf-cli/src/git_branch.rs:86`, `crates/fzf-cli/src/git_checkout.rs:77`, `crates/fzf-cli/src/git_commit.rs:409`, `crates/fzf-cli/src/git_status.rs:86`, `crates/fzf-cli/src/git_tag.rs:100`, `crates/git-lock/src/git.rs:17`, `crates/git-scope/src/git.rs:7`, `crates/semantic-commit/src/git.rs:3`, `crates/git-summary/src/git.rs:4` | High | Medium | M | adapt | Probe mechanism differs (`--git-dir` vs `--is-inside-work-tree`) and user messages are crate-specific; extract primitive checks, keep message adapters local. |
| G-2 | Git command wrappers (capture/trim/optional/pager/env/config injection) | `git` | Defs: `crates/git-lock/src/git.rs:27`, `crates/git-summary/src/git.rs:29`, `crates/git-scope/src/git_cmd.rs:6`, `crates/semantic-commit/src/staged_context.rs:289`, `crates/git-cli/src/commit_shared.rs:19`, `crates/git-cli/src/ci.rs:444`, `crates/git-cli/src/reset.rs:659` | High | High | L | adapt | This is the highest-risk cluster: trim policy, `GIT_PAGER` behavior, `core.quotepath=false`, and stderr/stdout formatting differ per CLI. |
| G-3 | Repo-root detection fallback | `git` | `crates/plan-tooling/src/repo_root.rs:4`, `crates/image-processing/src/util.rs:29`, `crates/codex-cli/src/agent/commit.rs:260` | Medium | Low | S | extract | Same behavior pattern (`git rev-parse --show-toplevel` -> fallback cwd) is already aligned enough for direct extraction. |
| C-1 | Best-effort clipboard copy | `clipboard` | Defs/calls: `crates/git-cli/src/clipboard.rs:7`, `crates/fzf-cli/src/defs/block_preview.rs:121`, usage in `crates/git-cli/src/commit.rs:139`, `crates/git-cli/src/commit_json.rs:134`, `crates/git-cli/src/utils.rs:92` | Medium | High | S | adapt | Tool priority and UX differ (git-cli supports `xsel` + warning text; fzf path is silent and omits `xsel`). Shared primitive + policy adapter keeps parity. |
| K-1 | Interactive/inherit-stdio git passthroughs | keep in crate | `crates/git-lock/src/git.rs:61`, `crates/git-scope/src/print.rs:144`, `crates/git-cli/src/ci.rs:461`, `crates/git-cli/src/reset.rs:676` | Medium | High | M | keep local | These helpers are tightly coupled to interactive output streams/prompts; extraction value is low versus parity risk in this phase. |

## Explicit extraction order (actionable)
1. `E-1` (`extract`): land shared truthy parser first; lowest coupling and high reuse.
2. `E-2` (`adapt`): add shared `NO_COLOR` baseline while preserving crate-specific color logic.
3. `P-1` (`adapt`): normalize command lookup onto `nils-common::process` + compatibility wrappers.
4. `S-1` + `S-2` (`adapt`): extract shell quote/ANSI primitives with explicit mode selection to preserve output text.
5. `C-1` (`adapt`): introduce clipboard primitive + policy adapters (`warn` vs `silent`, tool order).
6. `G-3` (`extract`): unify repo-root fallback helper (`rev-parse --show-toplevel` -> cwd).
7. `G-1` (`adapt`): unify git-repo probe primitives, keep per-CLI diagnostics and exit behavior local.
8. `P-2` (`adapt`): converge generic process capture after preceding low-risk modules are stable.
9. `G-2` (`adapt`, last): migrate git command wrappers only after characterization tests lock output/error parity.
10. `K-1` (`keep local`): defer or skip in this refactor stage.

## Task 1.2 contract snapshot (spec only)

### Proposed public API signatures
| Module | Proposed signatures | Notes |
| --- | --- | --- |
| `env` | `is_truthy(input: &str) -> bool`; `env_truthy(name: &str) -> bool`; `env_truthy_or(name: &str, default: bool) -> bool`; `no_color_enabled() -> bool` | Shared baseline only; extra color policy stays local. |
| `shell` | `quote_posix_single(input: &str) -> String`; `strip_ansi(input: &str, mode: AnsiStripMode) -> Cow<'_, str>` | `AnsiStripMode` allows parity-preserving parser differences. |
| `process` (expanded) | Existing: `cmd_exists`, `find_in_path`; New: `run_output`, `run_checked`, `run_stdout_trimmed` + `ProcessOutput` + `ProcessError` | `ProcessError::Io` for spawn/read failures; `ProcessError::NonZero` for non-zero exit with raw bytes. |
| `git` | `is_inside_work_tree(cwd) -> Result<bool, ProcessError>`; `repo_root(cwd) -> Result<Option<PathBuf>, ProcessError>`; `rev_parse(cwd, args) -> Result<String, ProcessError>`; `rev_parse_opt(cwd, args) -> Result<Option<String>, ProcessError>` | Repo probe and command primitives only; caller retains UX wording/policy. |
| `clipboard` | `copy_best_effort(text, policy) -> ClipboardOutcome` + `ClipboardPolicy` + `ClipboardTool` | Tool ordering and warn/silent behavior are adapter-driven policy. |

### Compatibility/adaptation rules (parity-sensitive)
- `nils-common` returns structured results and raw process data; caller crates render final warning/error text.
- Caller crates keep exit-code decisions (especially precondition failures and no-op/empty flows).
- Caller crates keep user-visible git command wording and shell snippet formatting.
- Caller crates keep clipboard warning strategy and tool-priority differences where they already diverge.
- `nils-common` APIs must remain domain-neutral and avoid embedding CLI-specific command logic.

## Parity-sensitive notes to keep explicit during migration
- Preserve exact warning/error copy and emoji prefixes (examples: `git-summary` repo warnings, `git-cli` clipboard warnings).
- Preserve exit code contracts (`semantic-commit` staged checks, git-repo precondition failures, no-staged-change flows).
- Preserve color behavior contracts (`--no-color`, `NO_COLOR`, terminal detection, starship-specific color env overrides).
- Preserve command snippet rendering contracts where shell quoting is user-visible.
