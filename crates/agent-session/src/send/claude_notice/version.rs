//! Tested Claude Code release range for the native notice module.
//!
//! The module relies on external loader rules and provider API semantics, so the
//! range is a reviewed list of releases that passed the module contract checks and
//! a native notice run. It never widens at runtime.
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct Version(u64, u64, u64);

pub(crate) const SUPPORTED_MIN: Version = Version(2, 1, 296);
// Raised only by a reviewed change citing the call-shape scan, `claude plugin
// validate` and a native notice run on that exact release.
pub(crate) const SUPPORTED_MAX: Version = Version(2, 1, 314);

impl Version {
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let mut parts = text.split('.').map(|part| {
            (!part.is_empty() && part.len() <= 9 && part.bytes().all(|b| b.is_ascii_digit()))
                .then(|| part.parse::<u64>().ok())
                .flatten()
        });
        let version = Self(parts.next()??, parts.next()??, parts.next()??);
        parts.next().is_none().then_some(version)
    }

    /// The `--version` banner of the supported external CLI: `X.Y.Z (Claude Code)`.
    pub(crate) fn parse_banner(banner: &str) -> Option<Self> {
        Self::parse(banner.trim().strip_suffix(" (Claude Code)")?)
    }

    pub(crate) fn in_range(self, min: Self, max: Self) -> bool {
        min <= self && self <= max
    }

    pub(crate) fn supported(self) -> bool {
        self.in_range(SUPPORTED_MIN, SUPPORTED_MAX)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// True when `text` names a release inside the supported range.
pub(crate) fn supported(text: &str) -> bool {
    Version::parse(text).is_some_and(Version::supported)
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn range_accepts_both_bounds_and_refuses_outside() {
        let (min, max) = (Version(2, 1, 296), Version(2, 1, 314));
        for (text, accepted) in [
            ("2.1.296", true),
            ("2.1.300", true),
            ("2.1.314", true),
            ("2.1.295", false),
            ("2.1.315", false),
            ("2.2.0", false),
            ("2.0.999", false),
            ("3.0.0", false),
        ] {
            assert_eq!(
                Version::parse(text).unwrap().in_range(min, max),
                accepted,
                "{text}"
            );
        }
    }

    #[test]
    fn parse_is_strict() {
        assert_eq!(Version::parse("2.1.296"), Some(Version(2, 1, 296)));
        for text in [
            "",
            "2",
            "2.1",
            "2.1.",
            "2.1.x",
            "2.1.296.1",
            "v2.1.296",
            "2.1.-1",
            " 2.1.296",
            "2.1.296 ",
            "2.1.1234567890",
        ] {
            assert_eq!(Version::parse(text), None, "{text:?}");
        }
        assert_eq!(
            Version::parse_banner("2.1.296 (Claude Code)\n"),
            Some(Version(2, 1, 296))
        );
        for text in [
            "2.1.296",
            "2.1.296 (Other)",
            "(Claude Code)",
            "x (Claude Code)",
        ] {
            assert_eq!(Version::parse_banner(text), None, "{text:?}");
        }
    }
}
