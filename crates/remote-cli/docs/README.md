# remote-cli command and output reference

```sh
remote-cli exec server --cwd '~/project' -- git status --short
remote-cli exec user@server --sudo --sudo-user operator -- id -u
remote-cli exec server -- printf '%s\n' 'a b' '$HOME' ''
remote-cli shell server --script-file ./check.sh
```

## Command arguments

`exec <host> [options] -- <program> [args...]` takes the host, working directory,
and remote argv as separate arguments. The `--` separator is mandatory.
Everything after it belongs to the remote program, including flags, empty
arguments, whitespace, quotes, dollar signs, and newlines. Quote values in the
calling shell to pass them unchanged. Tools rendering a command for an effect
classifier should retain this order and keep each value as its own argument.

`shell <host> [options] --script-file <file>` sends the local file to the remote
shell on stdin. Use this form when the input is shell syntax.

Both commands accept the options below.

| Option | Behavior |
| --- | --- |
| `--cwd DIR` | Start in the remote home by default. A leading `~/` or `$HOME/` resolves against the SSH user's remote home. The remainder is literal. Relative paths resolve from the login environment's working directory. |
| `--shell SHELL` | Select a remote shell executable for login startup and script interpretation. Defaults to the SSH user's login shell. The shell needs to support POSIX quoting, `-lc`, and `-s`. |
| `--sudo` | Run from the selected directory using `sudo -n --`. Existing sudo policy controls whether elevation succeeds. |
| `--connect-timeout S` | OpenSSH connect timeout in seconds, default `6`. Set `0` for the OpenSSH system default. |
| `--timeout S` | Local wall-clock deadline in seconds, default `40`. Set `0` to disable it. A deadline stops local OpenSSH and its process group. The remote command may continue. |
| `--format text\|json` | Output format, default `text`. |

`exec` also accepts `--sudo-user U` with `--sudo` to select a sudo user and
`--stdin-file FILE` to feed a local file to the command. Exec without a file
uses closed input. Input and script files must be regular files. OpenSSH
always uses `BatchMode=yes` and runs without a pseudo-terminal. Each
invocation attempts the operation once. SIGINT, SIGTERM, and SIGHUP stop
the local SSH process group and preserve captured output.

`remote-cli completion bash` and `remote-cli completion zsh` print completions.

## Output and exit status

Text mode writes the captured stdout and stderr to their respective channels
after the session ends. Local and transport failure diagnostics go to stderr.

JSON mode writes one envelope to stdout. The schema is
`cli.remote-cli.exec.v1` or `cli.remote-cli.shell.v1` and the `ok` field reflects
the exit status. `data` carries the fields below.

| Keys | Meaning |
| --- | --- |
| `host`, `requested_cwd`, `argv`, `script_file`, `sudo`, `sudo_user` | Requested inputs. An omitted cwd or sudo user is `null`. Only the applicable one of `argv` and `script_file` is populated. |
| `started_at`, `elapsed_ms`, `connect_timeout_ms`, `timeout_ms` | UTC start timestamp, monotonic elapsed milliseconds, and configured timeouts. |
| `ssh_exit_code`, `timed_out`, `signal` | Local OpenSSH exit code or terminating signal, plus whether the local deadline expired. A signaled process has a `null` SSH exit code. |
| `interrupted_by_signal` | Signal received by remote-cli when it stops local OpenSSH. Omitted outside interruptions. |
| `stdout`, `stderr` | Captured output, including partial output after failures. Non-UTF-8 bytes are replaced in JSON. |
| `remote_outcome` | `succeeded`, `failed`, or `unknown`. A connection failure, disconnect, or local deadline can leave the outcome unknown. |

The JSON result contains the supplied argv and command output. Keep
credentials out of arguments and avoid writing secrets to the captured streams.

| Exit status    | JSON error code                       | Meaning                                                                                               |
| -------------- | ------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| `0`            |                                       | Remote operation succeeded.                                                                           |
| Remote status  | `remote-exit`                         | Remote operation returned a nonzero status, including `124` or `255`.                                 |
| `255`          | `ssh-transport`                       | OpenSSH reported a connection or session error and did not confirm a remote exit of `255`.            |
| `124`          | `deadline-exceeded`                   | The local deadline expired.                                                                           |
| `128 + signal` | `interrupted`                         | The CLI received SIGINT, SIGTERM, or SIGHUP and stopped local OpenSSH. The remote outcome is unknown. |
| `128 + signal` | `ssh-signaled`                        | The local OpenSSH process was terminated by a signal before the deadline.                             |
| `64`           | `parse-error` or `unknown-subcommand` | CLI usage error, with schema `cli.remote-cli.error.v1`.                                               |
| `65`           | `input-file`                          | The input file could not be opened as a regular file.                                                 |
| `69`           | `ssh-unavailable`                     | The local SSH client could not be started.                                                            |
| `1`            | `local-io`                            | Local setup or capture failed.                                                                        |

A failed JSON envelope includes `error.code` and `error.message`. CLI parsing
and local setup errors can omit `data`.
