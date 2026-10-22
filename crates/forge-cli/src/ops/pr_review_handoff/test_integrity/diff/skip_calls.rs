//! JavaScript skip calls must start at an identifier boundary.

pub(super) fn contains(line: &str) -> bool {
    const MARKERS: &[&str] = &[
        "test.skip(",
        "it.skip(",
        "describe.skip(",
        "xit(",
        "xtest(",
        "xdescribe(",
    ];
    let mut previous = None;
    for (offset, character) in line.char_indices() {
        let boundary = previous.is_none_or(|c: char| !identifier(c));
        previous = Some(character);
        if !boundary || !matches!(character, 't' | 'i' | 'd' | 'x') {
            continue;
        }
        if MARKERS.iter().any(|marker| {
            let mut actual = line[offset..].chars().filter(|c| !c.is_whitespace());
            marker
                .chars()
                .all(|expected| actual.next() == Some(expected))
        }) {
            return true;
        }
    }
    false
}

fn identifier(character: char) -> bool {
    character.is_alphanumeric() || matches!(character, '_' | '$')
}
