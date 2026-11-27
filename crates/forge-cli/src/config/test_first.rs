//! Kind selection and declared test directories for test-first admission.

use nils_common::git::PrKind;

use super::ForgeConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestFirstPolicy {
    pub require: bool,
    pub require_kinds: Vec<PrKind>,
    /// Any invalid list in the layered config must be fixed before an enabled gate runs.
    pub invalid_require_kinds: bool,
    /// The fetched target base declared malformed additional test scopes.
    pub invalid_test_paths: bool,
}

impl Default for TestFirstPolicy {
    fn default() -> Self {
        Self {
            require: false,
            require_kinds: vec![PrKind::Feature, PrKind::Bug],
            invalid_require_kinds: false,
            invalid_test_paths: false,
        }
    }
}

impl TestFirstPolicy {
    pub fn required_for(&self, kind: PrKind) -> bool {
        self.require && self.require_kinds.contains(&kind)
    }
}

impl ForgeConfig {
    pub(crate) fn declares_test_path(&self, path: &str) -> bool {
        self.test_first_test_paths.iter().flatten().any(|scope| {
            let mut path_parts = path.split('/');
            scope.trim_end_matches('/').split('/').all(|part| {
                path_parts
                    .next()
                    .is_some_and(|name| part == "*" || part == name)
            }) && path_parts.next().is_some()
        })
    }

    pub fn resolve_test_first_policy(&self) -> TestFirstPolicy {
        TestFirstPolicy {
            require: self.resolve_test_first_required(None),
            invalid_require_kinds: self.warnings.iter().any(|warning| {
                warning.starts_with("invalid-config-value:test_first.require_kinds:")
            }),
            invalid_test_paths: false,
            require_kinds: self
                .test_first_require_kinds
                .clone()
                .unwrap_or_else(|| TestFirstPolicy::default().require_kinds),
        }
    }
}

pub(super) fn parse_test_paths(value: &toml::Value) -> Option<Vec<String>> {
    value
        .as_array()?
        .iter()
        .map(|value| {
            let pattern = value.as_str()?;
            let scope = pattern.strip_suffix('/')?;
            let parts = scope.split('/');
            let valid = parts.clone().any(|part| part != "*")
                && parts.into_iter().all(|part| {
                    part == "*"
                        || (!matches!(part, "" | "." | "..")
                            && part.bytes().all(|byte| {
                                byte.is_ascii_graphic() && !b"*?[]\\\"|".contains(&byte)
                            }))
                });
            valid.then(|| pattern.to_owned())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn parse(source: &str) -> ForgeConfig {
        ForgeConfig::parse_content(source).unwrap()
    }

    #[test]
    fn test_first_kinds_layering_overrides_lists_and_inherits_omissions() {
        let global = parse("[test_first]\nrequire = true\nrequire_kinds = [\"bug\"]");
        assert_eq!(
            global
                .clone()
                .overlaid_by(ForgeConfig::default())
                .resolve_test_first_policy(),
            global.resolve_test_first_policy()
        );
        for (settings, expected) in [
            ("require_kinds = []", vec![]),
            ("require_kinds = [\"feature\"]", vec![PrKind::Feature]),
            ("require_kinds = [\"bug\", 1]", vec![PrKind::Bug]),
        ] {
            let policy = global
                .clone()
                .overlaid_by(parse(&format!("[test_first]\n{settings}")))
                .resolve_test_first_policy();
            assert!(policy.require);
            assert_eq!(policy.require_kinds, expected);
        }
        let policy = global
            .overlaid_by(parse("[test_first]\nrequire = false"))
            .resolve_test_first_policy();
        assert!(!policy.required_for(PrKind::Bug));
    }
}
