"""Find workspace binaries that selected packages' tests run from other packages.

Cargo exports `CARGO_BIN_EXE_<name>` and builds binaries only for the package
under test, so a test that resolves another package's binary through
`nils_test_support::bin` or `nils_test_support::cmd` reads whatever
`target/<profile>` holds: nothing, or an artifact from an earlier release. A
package-scoped run must build those binaries itself (sympoies/nils-cli#2543).
"""
import pathlib
import re

# A literal binary name passed to a nils_test_support resolver. Matching a
# similarly named unrelated call only builds one extra binary.
RESOLVER_CALL = re.compile(
    r'\b(?:resolve|resolve_optional|run_resolved|sibling_or_skip|required_sibling)'
    r'\(\s*"([A-Za-z0-9_-]+)"')


def sibling_packages(packages, selected):
    """Map each unselected package whose binary a selected package resolves to why.

    `packages` are workspace package entries from `cargo metadata`; `selected`
    holds the names of the packages whose sources are scanned.
    """
    owners = {}
    roots = {}
    for package in packages:
        roots[package['name']] = pathlib.Path(package['manifest_path']).parent
        for target in package.get('targets', []):
            if 'bin' in target.get('kind', []):
                owners[target['name']] = package['name']

    siblings = {}
    for name in sorted(selected):
        for source in sorted(roots[name].rglob('*.rs')):
            for binary in RESOLVER_CALL.findall(source.read_text(errors='replace')):
                owner = owners.get(binary)
                if owner is not None and owner not in selected:
                    siblings.setdefault(owner, set()).add(f'{binary} resolved by {name}')
    return siblings
