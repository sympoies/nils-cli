use chrono::{Datelike, Duration, Local, NaiveDate};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateRange {
    pub label: String,
    pub start: String,
    pub end: String,
}

pub fn today_range() -> DateRange {
    today_range_for(Local::now().date_naive())
}

pub fn today_range_for(today: NaiveDate) -> DateRange {
    let date = format_date(today);
    DateRange {
        label: format!("today: {date}"),
        start: date.clone(),
        end: date,
    }
}

pub fn yesterday_range() -> DateRange {
    yesterday_range_for(Local::now().date_naive())
}

pub fn yesterday_range_for(today: NaiveDate) -> DateRange {
    let date = format_date(today - Duration::days(1));
    DateRange {
        label: format!("yesterday: {date}"),
        start: date.clone(),
        end: date,
    }
}

pub fn this_month_range() -> DateRange {
    this_month_range_for(Local::now().date_naive())
}

pub fn this_month_range_for(today: NaiveDate) -> DateRange {
    let start_date = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today);
    let start = format_date(start_date);
    let end = format_date(today);
    DateRange {
        label: format!("this month: {start} to {end}"),
        start,
        end,
    }
}

pub fn last_month_range() -> DateRange {
    last_month_range_for(Local::now().date_naive())
}

pub fn last_month_range_for(today: NaiveDate) -> DateRange {
    let first_this_month = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today);
    let end_last_month = first_this_month - Duration::days(1);
    let start_last_month =
        NaiveDate::from_ymd_opt(end_last_month.year(), end_last_month.month(), 1)
            .unwrap_or(end_last_month);
    let start = format_date(start_last_month);
    let end = format_date(end_last_month);
    DateRange {
        label: format!("last month: {start} to {end}"),
        start,
        end,
    }
}

pub fn this_week_range() -> DateRange {
    this_week_range_for(Local::now().date_naive())
}

pub fn this_week_range_for(today: NaiveDate) -> DateRange {
    let weekday = today.weekday().number_from_monday() as i64;
    let start = format_date(today - Duration::days(weekday - 1));
    let end = format_date(today + Duration::days(7 - weekday));
    DateRange {
        label: format!("this week: {start} to {end}"),
        start,
        end,
    }
}

pub fn last_week_range() -> DateRange {
    last_week_range_for(Local::now().date_naive())
}

pub fn last_week_range_for(today: NaiveDate) -> DateRange {
    let weekday = today.weekday().number_from_monday() as i64;
    let end = format_date(today - Duration::days(weekday));
    let start = format_date((today - Duration::days(weekday)) - Duration::days(6));
    DateRange {
        label: format!("last week: {start} to {end}"),
        start,
        end,
    }
}

pub fn format_date(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

pub fn validate_date(input: &str) -> Result<(), String> {
    if input.is_empty() {
        return Err("❌ Missing date value.".to_string());
    }
    if !is_date_format(input) {
        return Err(format!(
            "❌ Invalid date format: {input} (expected YYYY-MM-DD)."
        ));
    }
    if NaiveDate::parse_from_str(input, "%Y-%m-%d").is_err() {
        return Err(format!("❌ Invalid date value: {input}."));
    }
    Ok(())
}

pub fn is_date_format(input: &str) -> bool {
    let bytes = input.as_bytes();
    if bytes.len() != 10 {
        return false;
    }
    if bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    bytes
        .iter()
        .enumerate()
        .filter(|(idx, _)| *idx != 4 && *idx != 7)
        .all(|(_, b)| b.is_ascii_digit())
}

pub fn build_range_args(since: &str, until: &str) -> Vec<String> {
    let tz = Local::now().format("%z").to_string();
    let since_bound = format!("{since} 00:00:00 {tz}");
    let until_bound = format!("{until} 23:59:59 {tz}");
    vec![
        format!("--since={since_bound}"),
        format!("--until={until_bound}"),
        "--no-merges".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::{assert_eq, assert_ne};

    #[test]
    fn validate_date_accepts_valid_value() {
        assert_eq!(validate_date("2024-02-29"), Ok(()));
    }

    #[test]
    fn validate_date_rejects_empty() {
        assert_eq!(validate_date(""), Err("❌ Missing date value.".to_string()));
    }

    #[test]
    fn validate_date_rejects_bad_format() {
        assert_eq!(
            validate_date("2024/01/01"),
            Err("❌ Invalid date format: 2024/01/01 (expected YYYY-MM-DD).".to_string())
        );
    }

    #[test]
    fn validate_date_rejects_invalid_value() {
        assert_eq!(
            validate_date("2024-02-30"),
            Err("❌ Invalid date value: 2024-02-30.".to_string())
        );
    }

    #[test]
    fn date_format_checks_structure() {
        assert!(is_date_format("2024-01-31"));
        assert!(!is_date_format("2024-1-31"));
        assert!(!is_date_format("20240131"));
    }

    #[test]
    fn this_week_range_for_midweek() {
        let date = NaiveDate::from_ymd_opt(2024, 1, 3).unwrap();
        let range = this_week_range_for(date);
        assert_eq!(range.start, "2024-01-01");
        assert_eq!(range.end, "2024-01-07");
        assert_eq!(range.label, "this week: 2024-01-01 to 2024-01-07");
    }

    #[test]
    fn last_week_range_for_midweek() {
        let date = NaiveDate::from_ymd_opt(2024, 1, 3).unwrap();
        let range = last_week_range_for(date);
        assert_eq!(range.start, "2023-12-25");
        assert_eq!(range.end, "2023-12-31");
        assert_eq!(range.label, "last week: 2023-12-25 to 2023-12-31");
    }

    #[test]
    fn last_month_range_for_midmonth() {
        let date = NaiveDate::from_ymd_opt(2024, 3, 15).unwrap();
        let range = last_month_range_for(date);
        assert_eq!(range.start, "2024-02-01");
        assert_eq!(range.end, "2024-02-29");
        assert_eq!(range.label, "last month: 2024-02-01 to 2024-02-29");
    }

    #[test]
    fn build_range_args_includes_tz_suffix() {
        let args = build_range_args("2024-01-01", "2024-01-31");
        assert_eq!(args.len(), 3);
        assert!(args[0].starts_with("--since=2024-01-01 00:00:00 "));
        assert!(args[1].starts_with("--until=2024-01-31 23:59:59 "));

        for arg in &args[..2] {
            let tz = arg.split_whitespace().last().unwrap_or_default();
            assert_eq!(tz.len(), 5);
            assert!(tz.starts_with('+') || tz.starts_with('-'));
            assert!(tz[1..].chars().all(|c| c.is_ascii_digit()));
        }
        assert_ne!(args[2], "");
    }
}
