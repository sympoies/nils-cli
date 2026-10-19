//! CLI-only printable keys; native/web key-name contracts remain named-only.
use std::ffi::OsStr;

use clap::{
    ValueEnum,
    builder::{EnumValueParser, PossibleValue, TypedValueParser},
};

use super::SpecialKey;

/// Fixed-width UTF-8 storage keeps the key enum Copy for existing input paths.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PrintableKey {
    bytes: [u8; 4],
    len: usize,
}

impl PrintableKey {
    pub(super) fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len]).expect("encoded printable character")
    }
}

#[derive(Clone)]
pub(super) struct Parser;

impl TypedValueParser for Parser {
    type Value = SpecialKey;

    fn parse_ref(
        &self,
        command: &clap::Command,
        argument: Option<&clap::Arg>,
        value: &OsStr,
    ) -> Result<SpecialKey, clap::Error> {
        if let Some(text) = value.to_str() {
            let mut chars = text.chars();
            if let Some(ch) = chars.next()
                && !ch.is_control()
                && chars.next().is_none()
            {
                let mut bytes = [0; 4];
                let len = ch.encode_utf8(&mut bytes).len();
                return Ok(SpecialKey::Printable(PrintableKey { bytes, len }));
            }
        }
        EnumValueParser::<SpecialKey>::new().parse_ref(command, argument, value)
    }

    fn possible_values(&self) -> Option<Box<dyn Iterator<Item = PossibleValue> + '_>> {
        Some(Box::new(
            SpecialKey::value_variants()
                .iter()
                .filter_map(SpecialKey::to_possible_value),
        ))
    }
}

pub(crate) fn literal_args(key: SpecialKey) -> &'static [&'static str] {
    if matches!(key, SpecialKey::Printable(_)) {
        &["-l", "--"]
    } else {
        &[]
    }
}
