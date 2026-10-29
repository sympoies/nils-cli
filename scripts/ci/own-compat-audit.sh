#!/usr/bin/env bash
set -euo pipefail

# Repository-only ratchet, not a general source parser. See DEVELOPMENT.md.
# Python >=3.11 supplies the same stdlib TOML parser used by gate fixtures.
repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"
python3 - "$@" <<'PY'
import argparse
import calendar
from collections import Counter
import datetime
from pathlib import Path, PurePosixPath
import re
import subprocess
import sys

try:
    import tomllib
except ImportError:
    sys.exit('error: own compatibility audit requires Python >=3.11 (tomllib)')

parser = argparse.ArgumentParser(description='Audit existing own-version retention against exact expiring rows.')
parser.add_argument('--release', help='Also check the proposed X.Y.Z release before preparation.')
args = parser.parse_args()
baseline_path = Path('scripts/ci/own-compat-allowlist.toml')

# These exact files describe this detector and tombstones, not retained runtime
# behavior. Their synthetic positive/negative examples must not baseline the
# audit itself. Other tests, fixtures and CI scripts are scanned normally.
rule_files = {
    str(baseline_path), 'scripts/ci/own-compat-audit.sh',
    'scripts/ci/tests/own-compat-audit.test.sh', 'scripts/ci/docs-hygiene-audit.sh',
}

def active(path):
    p = PurePosixPath(path)
    if path in rule_files or p.name in {'THIRD_PARTY_LICENSES.md', 'THIRD_PARTY_NOTICES.md'} or any(part in {'target', 'node_modules', 'third_party',
                                         'third-party', 'vendor', '.git'} for part in p.parts):
        return False
    if any(p.parts[i:i+2] in {('docs', 'devlog'), ('docs', 'plans'), ('docs', 'discussions')}
           for i in range(len(p.parts) - 1)):
        return False
    return (p.parts[0] in {'crates', 'scripts', 'wrappers', 'completions', 'tests'}
            or path.startswith('.agents/scripts/')
            or path.startswith('.github/scripts/')
            or (len(p.parts) == 1 and p.suffix == '.md')
            or path.startswith(('docs/specs/', 'docs/runbooks/')))

# High-signal constructs only: identifier spellings, explicit retention
# declarations, serde/clap alias attributes, hidden JSON bool selectors and
# multi-version reader arms. A single current v1 constant is not a retired
# reader; neither are generic fallback/alias/migrate words.
identifier = re.compile(r'\b(?:legacy_[A-Za-z0-9_]+|[A-Za-z0-9_]+_legacy(?:_[A-Za-z0-9_]+)?)\b', re.I)
declaration = re.compile(r'backwards?[- ]compat(?:ible|ibility)?|compatibility (?:alias|shim|reader|fallback|no-op|allocator)|'
                         r'hidden alias for|alias for --format json|'
                         r'compatibility `[^`]+` (?:status|summary|field)|'
                         r'absent from older peers|older clients see the same envelope|'
                         r'(?:retain|keep|accept|read|support)(?:s|ed|ing)?[^\n.;]{0,70}(?:old|prior|retired) (?:schema|version|format|record|flag)', re.I)
attribute = re.compile(r'#\[(?:serde|arg|command|clap)\s*\(.*?\)\]', re.S)
alias = re.compile(r'\b(?:visible_)?alias(?:es)?\s*=\s*(?:"[^"]+"|\[[^\]]*\])', re.S)
builder_alias = re.compile(r'\.(?:visible_)?alias(?:es)?\(\s*(?:"[^"]+"|\[[^\]]*\])\s*\)', re.S)
manual_json_alias = re.compile(r'"--json"\s*=>\s*\{\s*\w+\.(?:output_)?format\s*=\s*OutputFormat::Json', re.S)
prohibition = re.compile(r'\b(?:never|must not|do not|reject\w*|refus\w*|unsupported|no longer|'
                         r'not (?:read|accept|retain)|no compatibility|no longer|there are no compatibility|'
                         r'rather than reading|remain removed|reintroduc\w*)\b', re.I)
negative_test = re.compile(r'(?:reject|refus|unsupported|cannot|never|does_not|do_not|is_not|not_read|'
                           r'no_longer|ignored|quarantin|invalid|corrupt|unknown|missing|fails_closed)', re.I)
multi_version = re.compile(r'(?:"[^"\n]*\.v\d+"|\b[A-Z_]*(?:SCHEMA|VERSION)[A-Z_0-9]*\b)'
                           r'\s*\|\s*(?:"[^"\n]*\.v\d+"|\b[A-Z_]*(?:SCHEMA|VERSION)[A-Z_0-9]*\b)')
python_versions = re.compile(r'(?:schema|version)[^\n]{0,45}\b(?:not )?in\s*\([^\n]*[\'"][^\'"\n]*\.v\d+[\'"][^\n]*,[^\n]*[\'"][^\'"\n]*\.v\d+[\'"][^\n]*\)')
conditional_versions = re.compile(
    r'(?P<field>\b[\w.]*schema_version(?:\.as_str\(\))?)\s*(?:==|!=)\s*'
    r'(?P<left>"[^"\n]*\.v\d+"|[A-Z_0-9]+)\s*(?:\|\||&&)\s*(?P=field)\s*(?:==|!=)\s*'
    r'(?P<right>"[^"\n]*\.v\d+"|[A-Z_0-9]+)')

def findings(text, markdown=False):
    ignored = set()
    # Only explicitly rejecting Rust test functions are non-retention examples.
    # Positive acceptance tests and helper/production functions remain scanned.
    for m in re.finditer(r'#\[(?:test|tokio::test)[^\]]*\]\s*(?:async\s+)?fn\s+(\w+)[^{]*\{', text):
        if (negative_test.search(m[1]) and 'legacy' in m[1].lower()
                and not re.search(r'(?:accepts?|supports?|reads?|recovers?)_legacy', m[1], re.I)):
            depth = 1
            end = m.end()
            while end < len(text) and depth:
                depth += (text[end] == '{') - (text[end] == '}')
                end += 1
            ignored.update(range(m.start(), end))
    hits = []

    def add(start, match):
        if start not in ignored:
            hits.append((start, ' '.join(match.split())))

    offset = 0
    for line in text.splitlines(keepends=True):
        stripped = line.lstrip()
        prose = markdown or stripped.startswith(('//', '# ', '- ', '|', '///')) or not any(
            c in line for c in '{};=')
        if not (prose and prohibition.search(line)):
            for m in identifier.finditer(line):
                if not re.search(r'(?:no|reject|refuse)_legacy|legacy_.*(?:rejected|refused)', m[0], re.I):
                    add(offset + m.start(), m[0])
            if declaration.search(line) and not re.search(r'\.expect_err\(|\.is_err\(', line):
                add(offset, line.strip())
        offset += len(line)
    for expression in (multi_version, python_versions, conditional_versions, builder_alias, manual_json_alias):
        for m in expression.finditer(text):
            literals = re.findall(r'"([^"\n]*)\.v\d+"', m[0])
            if len(literals) == 2 and literals[0] != literals[1]:
                continue  # Distinct current schema families, not older versions.
            add(m.start(), m[0])
    for m in attribute.finditer(text):
        for a in alias.finditer(m[0]):
            add(m.start() + a.start(), a[0])
        if re.search(r'\bhide\s*=\s*true', m[0]) and re.match(
                r'\s*(?:pub(?:\([^)]*\))?\s+)?json\s*:\s*bool', text[m.end():]):
            add(m.start(), m[0])
    # Deduplicate overlapping detectors, preserving occurrence multiplicity.
    return list(dict.fromkeys(hits))

def release(value, field):
    if not isinstance(value, str) or not re.fullmatch(r'\d+\.\d+\.\d+', value):
        raise ValueError(f'{field} must be a concrete X.Y.Z release')
    return tuple(map(int, value.split('.')))

def adjacent(a, b):
    return b in {(a[0], a[1], a[2]+1), (a[0], a[1]+1, 0), (a[0]+1, 0, 0)}

try:
    baseline = tomllib.loads(baseline_path.read_text())
    workspace = tomllib.loads(Path('Cargo.toml').read_text())
    current = release(workspace['workspace']['package']['version'], 'workspace version')
    proposed = release(args.release, '--release') if args.release else current
    paths = subprocess.check_output(['git', 'ls-files', '-z', '--cached', '--others',
                                     '--exclude-standard']).decode().split('\0')
    actual = Counter()
    locations = {}
    for path in sorted(set(filter(active, filter(None, paths)))):
        p = Path(path)
        if not p.exists() or not p.is_file() or p.is_symlink():
            continue
        try:
            text = p.read_text(encoding='utf-8')
        except UnicodeDecodeError:
            continue  # Images/binaries carry no source declarations.
        for start, match in findings(text, p.suffix == '.md'):
            key = (path, match)
            actual[key] += 1
            locations.setdefault(key, text[:start].count('\n') + 1)
except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
    print(f'error: own compatibility audit TOML/input: {error}', file=sys.stderr)
    sys.exit(2)

errors = []
rows = baseline.get('retention', [])
if type(baseline.get('schema_version')) is not int or baseline['schema_version'] != 1 or set(baseline) - {'schema_version', 'retention'} or not isinstance(rows, list):
    sys.exit('error: malformed own compatibility baseline schema')
seen = set()
required = {'path', 'match', 'kind', 'reason', 'issue', 'expires', 'count'}
optional = {'previous_release', 'current_release', 'first_disallowed_release', 'cross_host', 'external_version'}
today = datetime.datetime.now(datetime.timezone.utc).date()
for index, row in enumerate(rows, 1):
    label = f'baseline row {index}'
    try:
        if not isinstance(row, dict):
            raise ValueError('row must be a table')
        missing = required - row.keys()
        if missing or row.keys() - required - optional:
            raise ValueError(f'missing/unknown fields: {sorted(missing or row.keys() - required - optional)}')
        for field in required - {'count'}:
            if not isinstance(row[field], str) or not row[field].strip():
                raise ValueError(f'{field} must be nonempty text')
        path = PurePosixPath(row['path'])
        if path.is_absolute() or '..' in path.parts or str(path) != row['path'] or not active(str(path)):
            raise ValueError('path must name one exact active file')
        if not re.fullmatch(r'https://github.com/sympoies/nils-cli/issues/[1-9]\d*', row['issue']):
            raise ValueError('issue must be the owning public nils-cli removal/classification issue')
        if type(row['count']) is not int or row['count'] < 1:
            raise ValueError('count must be a positive integer')
        try:
            expiry = datetime.date.fromisoformat(row['expires'])
        except ValueError as error:
            raise ValueError(f'expires: {error}') from error
        if expiry.isoformat() != row['expires']:
            raise ValueError('expires must be YYYY-MM-DD')
        if expiry <= today:
            raise ValueError(f'expired on {expiry}')
        key = (row['path'], row['match'])
        if key in seen:
            raise ValueError('duplicate path+match')
        seen.add(key)
        if row['kind'] not in {'removal-scheduled', 'current-contract', 'external', 'rollback-window'}:
            raise ValueError('unknown kind')
        allowed = {'first_disallowed_release'}
        if row['kind'] == 'external':
            allowed.add('external_version')
        if row['kind'] == 'rollback-window':
            allowed.update({'previous_release', 'current_release', 'cross_host'})
        if row.keys() & optional - allowed:
            raise ValueError('fields do not belong to the declared kind')
        if row['kind'] == 'external':
            external = row.get('external_version', '')
            if not isinstance(external, str) or not re.search(r'\d', external) or not re.search(r'[A-Za-z]', external):
                raise ValueError('external_version must name the supported external version')
            month = today.month + 6
            year = today.year + (month - 1) // 12
            month = (month - 1) % 12 + 1
            review_limit = today.replace(year=year, month=month,
                                         day=min(today.day, calendar.monthrange(year, month)[1]))
            if expiry > review_limit:
                raise ValueError('external expires exceeds the six-month review limit')
        if row['kind'] == 'rollback-window':
            previous = release(row.get('previous_release'), 'previous_release')
            live = release(row.get('current_release'), 'current_release')
            fence = release(row.get('first_disallowed_release'), 'first_disallowed_release')
            if not adjacent(previous, live) or not adjacent(live, fence):
                raise ValueError('rollback-window must name adjacent releases and the next release fence')
            cross_host = row.get('cross_host', '')
            if (not isinstance(cross_host, str) or not re.search(r'cross.host|across hosts', cross_host, re.I)
                    or not re.search(r'wire|state|journal|sender|receiver', cross_host, re.I)):
                raise ValueError('cross_host must describe actual cross-host wire/state interoperability')
        if 'first_disallowed_release' in row:
            fence = release(row['first_disallowed_release'], 'first_disallowed_release')
            if max(current, proposed) >= fence:
                raise ValueError(f'release fence {row["first_disallowed_release"]} reached')
        if actual[key] == 0:
            raise ValueError(f'stale row: {key}')
        if actual[key] != row['count']:
            raise ValueError(f'count changed for {key}: baseline={row["count"]}, actual={actual[key]}')
    except (ValueError, TypeError) as error:
        errors.append(f'{label}: {error}')
for path, match in sorted(actual.keys() - seen):
    errors.append(f'unlisted {path}:{locations[path, match]}: {match} (count={actual[path, match]})')
for error in errors:
    print(f'FAIL: {error}')
if errors:
    sys.exit(1)
print(f'PASS: own compatibility audit ({len(actual)} exact matches, {len(rows)} expiring rows)')
PY
