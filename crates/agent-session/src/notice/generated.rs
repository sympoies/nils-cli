//! Formatting owned by internal generators; caller bodies never use this path.
pub(crate) fn body(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let (content, ending) = if let Some(content) = line.strip_suffix("\r\n") {
            (content, "\r\n")
        } else if let Some(content) = line.strip_suffix('\n') {
            (content, "\n")
        } else {
            (line, "")
        };
        result.push_str(content.trim_end_matches(' '));
        result.push_str(ending);
    }
    result
}
