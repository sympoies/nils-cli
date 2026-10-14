//! OSC-8 metadata affects hyperlinks, not visible cells or SGR state.

pub(super) fn consume(chars: &mut impl Iterator<Item = char>) -> Option<()> {
    let mut payload = String::new();
    loop {
        match chars.next()? {
            '\u{7}' => break,
            '\u{1b}' => {
                if chars.next()? != '\\' {
                    return None;
                }
                break;
            }
            ch if !ch.is_control() && payload.len() + ch.len_utf8() <= 4096 => {
                payload.push(ch);
            }
            _ => return None,
        }
    }
    let mut fields = payload.splitn(3, ';');
    if fields.next()? != "8" {
        return None;
    }
    fields.next()?;
    fields.next()?;
    Some(())
}
