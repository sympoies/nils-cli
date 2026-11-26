# remote-cli

Run a command with literal arguments or a local script on a host via OpenSSH.

Requires `ssh` on PATH and an OpenSSH server with a POSIX-compatible login
shell and `/bin/sh`. Use an OpenSSH alias or `user@host`. The client uses normal
SSH configuration, authentication, host-key checks, and ProxyJump.

See the [command and output reference](docs/README.md).
