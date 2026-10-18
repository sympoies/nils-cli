//! Kind selection for the opt-in test-first evidence gate.

use nils_common::git::PrKind;

use super::ForgeConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestFirstPolicy {
    pub require: bool,
    pub require_kinds: Vec<PrKind>,
}

impl Default for TestFirstPolicy {
    fn default() -> Self {
        Self {
            require: false,
            require_kinds: vec![PrKind::Feature, PrKind::Bug],
        }
    }
}

impl TestFirstPolicy {
    pub fn required_for(&self, kind: PrKind) -> bool {
        self.require && self.require_kinds.contains(&kind)
    }
}

impl ForgeConfig {
    pub fn resolve_test_first_policy(&self) -> TestFirstPolicy {
        TestFirstPolicy {
            require: self.resolve_test_first_required(None),
            require_kinds: self
                .test_first_require_kinds
                .clone()
                .unwrap_or_else(|| TestFirstPolicy::default().require_kinds),
        }
    }
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
