# Research CLI completion contract

The clap command graph owns `search`, `read-url`, `brief`, `serve`, `completion`
and help, including provider/output/shell enum values and all limit flags.
There are no aliases or dynamic value providers. Bash and zsh use the same
static clap_complete generation path; generation failure cannot select another
completion implementation.

Enforcement metadata:

- `completion_mode=clap-first`
- `completion_mode_toggles=forbidden`
- `alternate_completion_dispatch=forbidden`
- `generated_load_failure=fail-closed`
- `completion_engine=static`

The CLI tests exercise both shell exports and key flags. The completion
freshness and flag-parity audits compare the generated distribution assets;
shell syntax checks cover both committed files. These are canonical completion
instructions for this crate, rather than a development migration record.
