//! Resolve the short hostname without depending on an external utility.

pub(super) fn short_hostname() -> Option<String> {
    // 256 bytes covers the supported Unix hosts (Linux and macOS).
    let mut bytes = [0_u8; 256];
    // SAFETY: bytes is writable for the exact length passed to gethostname.
    if unsafe { libc::gethostname(bytes.as_mut_ptr().cast(), bytes.len()) } != 0 {
        return None;
    }
    let end = bytes.iter().position(|&byte| byte == 0)?;
    short_name(&bytes[..end])
}

fn short_name(bytes: &[u8]) -> Option<String> {
    let name = String::from_utf8_lossy(bytes);
    let short = name.split('.').next()?.trim();
    (!short.is_empty()).then(|| short.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn hostname_is_short_and_nonempty() {
        assert_eq!(short_name(b"fixture.example"), Some("fixture".into()));
        assert_eq!(short_name(b"fixture"), Some("fixture".into()));
        assert_eq!(short_name(b""), None);
    }
}
