use chrono::{Local, NaiveDateTime, TimeZone};

pub const TIMESTAMP_PREFIX: &str = "timestamp=";

#[derive(Debug, Clone)]
pub struct LockFile {
    pub hash: String,
    pub note: String,
    pub timestamp: Option<String>,
}

pub fn parse_lock_file(content: &str) -> LockFile {
    let mut lines = content.lines();
    let line1 = lines.next().unwrap_or("");
    let (hash, note) = parse_lock_line(line1);

    let mut timestamp = None;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix(TIMESTAMP_PREFIX) {
            let trimmed = rest.trim();
            if !trimmed.is_empty() {
                timestamp = Some(trimmed.to_string());
            }
            break;
        }
    }

    LockFile {
        hash,
        note,
        timestamp,
    }
}

pub fn parse_lock_line(line: &str) -> (String, String) {
    let mut parts = line.splitn(2, '#');
    let hash = parts.next().unwrap_or("").trim().to_string();
    let note = parts.next().unwrap_or("").trim().to_string();
    (hash, note)
}

pub fn timestamp_epoch(timestamp: &str) -> i64 {
    if timestamp.trim().is_empty() {
        return 0;
    }

    if let Ok(parsed) = NaiveDateTime::parse_from_str(timestamp.trim(), "%Y-%m-%d %H:%M:%S")
        && let Some(local) = Local.from_local_datetime(&parsed).single()
    {
        return local.timestamp();
    }

    0
}

#[cfg(test)]
mod tests {
    use super::{TIMESTAMP_PREFIX, parse_lock_file, parse_lock_line, timestamp_epoch};
    use chrono::{Local, NaiveDateTime, TimeZone};
    use pretty_assertions::assert_eq;

    #[test]
    fn parse_lock_line_splits_hash_and_note() {
        let (hash, note) = parse_lock_line("abc123 # before refactor");
        assert_eq!(hash, "abc123");
        assert_eq!(note, "before refactor");
    }

    #[test]
    fn parse_lock_line_trims_whitespace() {
        let (hash, note) = parse_lock_line("  abc123   #   note here   ");
        assert_eq!(hash, "abc123");
        assert_eq!(note, "note here");
    }

    #[test]
    fn parse_lock_file_reads_timestamp_line() {
        let content = format!("abc123 # note\n{TIMESTAMP_PREFIX}2020-01-01 00:00:00\n");
        let lock = parse_lock_file(&content);
        assert_eq!(lock.hash, "abc123");
        assert_eq!(lock.note, "note");
        assert_eq!(lock.timestamp, Some("2020-01-01 00:00:00".to_string()));
    }

    #[test]
    fn timestamp_epoch_returns_zero_for_invalid() {
        assert_eq!(timestamp_epoch(""), 0);
        assert_eq!(timestamp_epoch("not-a-date"), 0);
    }

    #[test]
    fn timestamp_epoch_parses_valid() {
        let ts = "2020-01-02 03:04:05";
        let parsed = NaiveDateTime::parse_from_str(ts, "%Y-%m-%d %H:%M:%S").expect("parse");
        let expected = Local
            .from_local_datetime(&parsed)
            .single()
            .expect("local time");
        assert_eq!(timestamp_epoch(ts), expected.timestamp());
    }
}
