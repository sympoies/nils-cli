## 2026-10-30 - Plain absolute-path prompts are no longer owner commands
<!-- devlog-id: 2026-10-30-issue-3062-path-first-token -->

### Result

- owner_commands::parse recognizes a command only when the first token starts
  with a slash and has no further slash, so an absolute-path prompt (with
  optional caption) follows the caller's require_idle policy instead of being
  forced idle-only
- A namespaced /ns/cmd first token is ordinary text by the same rule; a path as
  a command argument keeps the command guarded

### Why / context

- A leading slash alone made any prompt an owner command, so a plain upload
  path was refused busy while a Claude turn was working
- Known-command-plus-prose handling is unchanged and awaits a separate reviewed
  contract

### Evidence

- Behavioral HTTP regression native_busy_reply
  absolute_path_prompt_is_a_plain_busy_reply_but_slash_commands_stay_guarded:
  red on the baseline (409 busy for the plain path), green after;
  genuine-command controls were not reached in the red run

### Links

- <https://github.com/sympoies/nils-cli/issues/3062>
