#!/usr/bin/env bash
set -euo pipefail

# Black-box repositories exercise the real audit and its fixed baseline owner.
repo_root="$(git rev-parse --show-toplevel)"
source "$repo_root/scripts/dev-env.sh"
python3 - "$repo_root" <<'PY'
import datetime
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

audit = Path(sys.argv[1]) / "scripts/ci/own-compat-audit.sh"
with tempfile.TemporaryDirectory(prefix="own-compat-audit-") as temporary:
    root = Path(temporary)
    subprocess.run(["git", "init", "-q", str(root)], check=True)
    (root / "scripts/ci").mkdir(parents=True)
    shutil.copyfile(audit, root / "scripts/ci/own-compat-audit.sh")
    (root / "Cargo.toml").write_text('[workspace.package]\nversion = "1.38.1"\n')
    source = root / "crates/example/src/lib.rs"
    source.parent.mkdir(parents=True)
    baseline = root / "scripts/ci/own-compat-allowlist.toml"
    future = (datetime.date.today() + datetime.timedelta(days=30)).isoformat()

    def write_rows(rows):
        text = "schema_version = 1\n"
        for row in rows:
            text += "\n[[retention]]\n"
            for key, value in row.items():
                text += f"{key} = {json.dumps(value)}\n"
        baseline.write_text(text)

    def row(match='LEGACY_X', **extra):
        return dict(path='crates/example/src/lib.rs', match=match,
                    kind='removal-scheduled', reason='Existing reader removal in the owning slice.',
                    issue='https://github.com/sympoies/nils-cli/issues/3081',
                    expires=future, count=1, **extra)

    def run(label, expected, needle, *args):
        result = subprocess.run(['bash', 'scripts/ci/own-compat-audit.sh', *args],
                                cwd=root, text=True, capture_output=True)
        output = result.stdout + result.stderr
        if result.returncode != expected or needle not in output:
            raise AssertionError(f'{label}: exit {result.returncode}, expected {expected}\n{output}')
        print(f'ok: {label}')

    write_rows([])
    source.write_text('const LEGACY_X: u32 = 1;\n')
    run('identifier missed by the original guard', 1, 'unlisted')
    write_rows([row()])
    run('valid scheduled debt', 0, 'PASS: own compatibility audit')
    source.write_text('const LEGACY_X: u32 = 1;\nconst SECOND: u32 = LEGACY_X;\n')
    run('baseline cannot admit another occurrence', 1, 'count')
    source.write_text('const CURRENT: u32 = 1;\n')
    run('removed occurrence makes row stale', 1, 'stale')

    source.write_text('const LEGACY_X: u32 = 1;\n')
    malformed = row()
    del malformed['reason']
    write_rows([malformed])
    run('missing reason', 1, 'reason')
    malformed = row()
    malformed['issue'] = 'private-or-unowned'
    write_rows([malformed])
    run('owning public issue required', 1, 'issue')
    malformed = row()
    malformed['expires'] = '2026-02-30'
    write_rows([malformed])
    run('invalid calendar date', 1, 'expires')
    malformed['expires'] = '2000-01-01'
    write_rows([malformed])
    run('hard expiry', 1, 'expired')
    baseline.write_text('schema_version = 1\n[[retention]\n')
    run('malformed TOML', 2, 'TOML')

    window = row(previous_release='1.38.0', current_release='1.38.1',
                 first_disallowed_release='1.38.2',
                 cross_host='Federated sender and receiver exchange the journal across hosts.')
    window['kind'] = 'rollback-window'
    write_rows([window])
    run('adjacent cross-host release pair', 0, 'PASS: own compatibility audit')
    run('concrete release fence', 1, 'release fence', '--release', '1.38.2')
    local = dict(window, cross_host='local restore and replay')
    write_rows([local])
    run('local rollback is ineligible', 1, 'cross_host')
    write_rows([dict(window, previous_release='1.36.0')])
    run('nonadjacent release pair', 1, 'adjacent')
    write_rows([dict(window, first_disallowed_release='next-release')])
    run('rolling release fence is invalid', 1, 'first_disallowed_release')

    external = row(external_version='Example provider API 2026-10-01')
    external['kind'] = 'external'
    write_rows([external])
    run('versioned external support', 0, 'PASS: own compatibility audit')
    del external['external_version']
    write_rows([external])
    run('unversioned external support', 1, 'external_version')
    external['external_version'] = 'Example provider API 2026-10-01'
    external['expires'] = (datetime.date.today() + datetime.timedelta(days=200)).isoformat()
    write_rows([external])
    run('external support has a bounded review date', 1, 'six-month')

    write_rows([])
    source.write_text('// fallback, alias, migrate, compatibility are generic words.\n'
                      'const REGISTRY_VERSION: &str = "example.registry.v1";\n'
                      '#[test]\nfn rejects_legacy_input() {\n'
                      '    assert!(read("legacy_input").is_err());\n}\n')
    run('generic words, current default and explicit rejection', 0, 'PASS: own compatibility audit')
    source.write_text('#[test]\nfn accepts_legacy_input() {\n'
                      '    assert!(read("legacy_input").is_ok());\n}\n')
    run('positive acceptance tests remain scanned', 1, 'unlisted')
    source.write_text('#[test]\nfn rejects_unknown_but_accepts_legacy() {\n'
                      '    assert!(read("legacy_input").is_ok());\n}\n')
    run('mixed acceptance test is not a rejection exemption', 1, 'unlisted')
    source.write_text('struct Current {\n    #[serde(\n        alias = "old_name"\n    )]\n'
                      '    current_name: String,\n}\n')
    run('disposable serde alias mutation fails real audit', 1, 'unlisted')
    source.write_text('#[command(visible_alias = "old-command")]\nstruct Cli;\n')
    run('clap alias construct', 1, 'unlisted')
    source.write_text('Arg::new("current").visible_alias("old");\n')
    run('clap builder alias construct', 1, 'unlisted')
    source.write_text('#[arg(long, hide = true)]\njson: bool,\n')
    run('hidden JSON flag construct', 1, 'unlisted')
    source.write_text('"--json" => { options.format = OutputFormat::Json; }\n')
    run('hand-parsed format alias', 1, 'unlisted')
    source.write_text('if matches!(schema_version, "example.v1" | "example.v2") {}\n')
    run('reader accepting multiple versions', 1, 'unlisted')
    source.write_text('if schema_version == "example.v1" || schema_version == "example.v2" {}\n')
    run('conditional version reader', 1, 'unlisted')
    source.write_text('// Keep backward-compatible readers for older records.\n')
    run('explicit retention declaration', 1, 'unlisted')
    source.write_text('// Never keep backward-compatible readers.\n')
    run('prohibition text', 0, 'PASS: own compatibility audit')
    source.write_text('')
    (root / 'docs/devlog').mkdir(parents=True)
    (root / 'docs/devlog/history.md').write_text('Existing LEGACY_X provenance.\n')
    (root / 'docs/plans').mkdir(parents=True)
    (root / 'docs/plans/old.md').write_text('Existing LEGACY_X plan.\n')
    run('immutable devlog and plan provenance', 0, 'PASS: own compatibility audit')

print('PASS: own-compat-audit.test.sh')
PY
