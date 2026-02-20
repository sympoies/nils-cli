use std::path::Path;

#[derive(Clone, Debug)]
pub struct CacheEntry {
    pub fetched_at_epoch: i64,
    pub non_weekly_label: String,
    pub non_weekly_remaining: i64,
    pub non_weekly_reset_epoch: Option<i64>,
    pub weekly_remaining: i64,
    pub weekly_reset_epoch: i64,
}

pub fn read_cache_file(path: &Path) -> Option<CacheEntry> {
    let content = std::fs::read_to_string(path).ok()?;
    parse_cache_kv(&content)
}

fn parse_cache_kv(content: &str) -> Option<CacheEntry> {
    let mut fetched_at_epoch: Option<i64> = None;
    let mut non_weekly_label: Option<String> = None;
    let mut non_weekly_remaining: Option<i64> = None;
    let mut non_weekly_reset_epoch: Option<i64> = None;
    let mut weekly_remaining: Option<i64> = None;
    let mut weekly_reset_epoch: Option<i64> = None;

    for line in content.lines() {
        if let Some(value) = line.strip_prefix("fetched_at=") {
            fetched_at_epoch = value.parse::<i64>().ok();
        } else if let Some(value) = line.strip_prefix("non_weekly_label=") {
            non_weekly_label = Some(value.to_string());
        } else if let Some(value) = line.strip_prefix("non_weekly_remaining=") {
            non_weekly_remaining = value.parse::<i64>().ok();
        } else if let Some(value) = line.strip_prefix("non_weekly_reset_epoch=") {
            non_weekly_reset_epoch = value.parse::<i64>().ok();
        } else if let Some(value) = line.strip_prefix("weekly_remaining=") {
            weekly_remaining = value.parse::<i64>().ok();
        } else if let Some(value) = line.strip_prefix("weekly_reset_epoch=") {
            weekly_reset_epoch = value.parse::<i64>().ok();
        }
    }

    let fetched_at_epoch = fetched_at_epoch?;
    let non_weekly_label = non_weekly_label?;
    if non_weekly_label.trim().is_empty() {
        return None;
    }
    let non_weekly_remaining = non_weekly_remaining?;
    let weekly_remaining = weekly_remaining?;
    let weekly_reset_epoch = weekly_reset_epoch?;

    Some(CacheEntry {
        fetched_at_epoch,
        non_weekly_label,
        non_weekly_remaining,
        non_weekly_reset_epoch,
        weekly_remaining,
        weekly_reset_epoch,
    })
}

pub fn render_line(
    entry: &CacheEntry,
    prefix: &str,
    show_5h: bool,
    weekly_reset_time_format: &str,
) -> Option<String> {
    let weekly_reset_time = crate::rate_limits::render::format_epoch_local(
        entry.weekly_reset_epoch,
        weekly_reset_time_format,
    )
    .unwrap_or_else(|| "?".to_string());

    let weekly_token = format!("W:{}%", entry.weekly_remaining);

    if show_5h {
        let non_weekly_token =
            format!("{}:{}%", entry.non_weekly_label, entry.non_weekly_remaining);
        return Some(format!(
            "{prefix}{non_weekly_token} {weekly_token} {weekly_reset_time}"
        ));
    }

    Some(format!("{prefix}{weekly_token} {weekly_reset_time}"))
}
