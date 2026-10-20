#!/usr/bin/env python3
"""Keep nextest reports and recorded history independent of Cargo caches."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import uuid


def run(arguments):
    repo = Path(__file__).resolve().parents[2]
    configured = os.environ.get('NILS_CLI_REPORT_DIR')
    if configured:
        output = Path(configured).expanduser().resolve()
    else:
        state = Path(os.environ.get('XDG_STATE_HOME', Path.home() / '.local/state'))
        owner = hashlib.sha256(str(repo).encode()).hexdigest()[:16]
        output = (state / 'nils-cli/reports' / owner / uuid.uuid4().hex).resolve()
    targets = [repo / 'target']
    if os.environ.get('CARGO_TARGET_DIR'):
        targets.append(Path(os.environ['CARGO_TARGET_DIR']).resolve())
    if any(output == target.resolve() or target.resolve() in output.parents for target in targets):
        raise ValueError('NILS_CLI_REPORT_DIR must be outside Cargo target directories')
    output.mkdir(parents=True, exist_ok=True)
    config = output / 'nextest-reports.toml'
    config.write_text('[store]\ndir = ' + json.dumps(str(output / 'nextest')) + '\n')
    environment = dict(os.environ, NEXTEST_STATE_DIR=str(output / 'history'))
    print(f'gate reports: {output}', flush=True)
    # Tool defaults preserve all repository profiles, filters, and timeouts.
    separator = arguments.index('--') if '--' in arguments else len(arguments)
    command = [*arguments[:separator], '--tool-config-file', f'nils-gate:{config}',
               *arguments[separator:]]
    return subprocess.call(command, env=environment)


if __name__ == '__main__':
    try:
        sys.exit(run(sys.argv[1:]))
    except (OSError, ValueError) as error:
        print(f'error: gate report routing failed: {error}', file=sys.stderr)
        sys.exit(2)
