pub fn py_repr(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\x00"),
            c if c.is_control() => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

pub fn py_list_repr(items: &[String]) -> String {
    let mut out = String::from("[");
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&py_repr(item));
    }
    out.push(']');
    out
}

#[cfg(test)]
mod tests {
    use super::{py_list_repr, py_repr};
    use pretty_assertions::assert_eq;

    #[test]
    fn py_repr_escapes_control_and_quotes() {
        let raw = "a'b\\c\n\r\t";
        assert_eq!(py_repr(raw), "'a\\'b\\\\c\\n\\r\\t'");
    }

    #[test]
    fn py_repr_escapes_nul_byte() {
        let raw = format!("a{}b", '\0');
        assert_eq!(py_repr(&raw), "'a\\x00b'");
    }

    #[test]
    fn py_list_repr_wraps_items_like_python_list_literals() {
        let items = vec!["a".to_string(), "b c".to_string(), "d'e".to_string()];
        assert_eq!(py_list_repr(&items), "['a', 'b c', 'd\\'e']");
    }
}
