use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub struct UsageData {
    pub primary: Window,
    pub secondary: Window,
}

pub struct Window {
    pub limit_window_seconds: i64,
    pub used_percent: f64,
    pub reset_at: i64,
}

pub struct RenderValues {
    pub primary_label: String,
    pub secondary_label: String,
    pub primary_remaining: i64,
    pub secondary_remaining: i64,
    pub primary_reset_epoch: i64,
    pub secondary_reset_epoch: i64,
}

pub struct WeeklyValues {
    pub weekly_remaining: i64,
    pub weekly_reset_epoch: i64,
    pub non_weekly_label: String,
    pub non_weekly_remaining: i64,
    pub non_weekly_reset_epoch: Option<i64>,
}

pub fn parse_usage(body: &str) -> Option<UsageData> {
    parse_usage_body(body)
}

pub fn parse_usage_body(body: &str) -> Option<UsageData> {
    static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

    let mut path = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    path.push(format!(
        "gemini-rate-limits-usage-{}-{nanos}-{seq}.json",
        std::process::id(),
    ));

    fs::write(&path, body).ok()?;
    let value = crate::json::read_json(&path).ok();
    let _ = fs::remove_file(&path);
    let value = value?;

    let rate_limit = value.get("rate_limit")?;
    let primary_raw = rate_limit.get("primary_window")?;
    let secondary_raw = rate_limit.get("secondary_window")?;

    let primary = Window {
        limit_window_seconds: primary_raw.get("limit_window_seconds")?.as_i64()?,
        used_percent: primary_raw
            .get("used_percent")
            .and_then(|value| value.as_f64())
            .unwrap_or(0.0),
        reset_at: primary_raw.get("reset_at")?.as_i64()?,
    };
    let secondary = Window {
        limit_window_seconds: secondary_raw.get("limit_window_seconds")?.as_i64()?,
        used_percent: secondary_raw
            .get("used_percent")
            .and_then(|value| value.as_f64())
            .unwrap_or(0.0),
        reset_at: secondary_raw.get("reset_at")?.as_i64()?,
    };

    Some(UsageData { primary, secondary })
}

pub fn render_values(data: &UsageData) -> RenderValues {
    let primary_label = format_window_seconds(data.primary.limit_window_seconds)
        .unwrap_or_else(|| "Primary".to_string());
    let secondary_label = format_window_seconds(data.secondary.limit_window_seconds)
        .unwrap_or_else(|| "Secondary".to_string());

    let primary_remaining = remaining_percent(data.primary.used_percent);
    let secondary_remaining = remaining_percent(data.secondary.used_percent);

    RenderValues {
        primary_label,
        secondary_label,
        primary_remaining,
        secondary_remaining,
        primary_reset_epoch: data.primary.reset_at,
        secondary_reset_epoch: data.secondary.reset_at,
    }
}

pub fn weekly_values(values: &RenderValues) -> WeeklyValues {
    let (
        weekly_remaining,
        weekly_reset_epoch,
        non_weekly_label,
        non_weekly_remaining,
        non_weekly_reset_epoch,
    ) = if values.primary_label == "Weekly" {
        (
            values.primary_remaining,
            values.primary_reset_epoch,
            values.secondary_label.clone(),
            values.secondary_remaining,
            Some(values.secondary_reset_epoch),
        )
    } else {
        (
            values.secondary_remaining,
            values.secondary_reset_epoch,
            values.primary_label.clone(),
            values.primary_remaining,
            Some(values.primary_reset_epoch),
        )
    };

    WeeklyValues {
        weekly_remaining,
        weekly_reset_epoch,
        non_weekly_label,
        non_weekly_remaining,
        non_weekly_reset_epoch,
    }
}

pub fn format_window_seconds(raw: i64) -> Option<String> {
    if raw <= 0 {
        return None;
    }
    if raw % 604_800 == 0 {
        let weeks = raw / 604_800;
        if weeks == 1 {
            return Some("Weekly".to_string());
        }
        return Some(format!("{weeks}w"));
    }
    if raw % 86_400 == 0 {
        return Some(format!("{}d", raw / 86_400));
    }
    if raw % 3_600 == 0 {
        return Some(format!("{}h", raw / 3_600));
    }
    if raw % 60 == 0 {
        return Some(format!("{}m", raw / 60));
    }
    Some(format!("{raw}s"))
}

pub fn format_epoch_local_datetime(epoch: i64) -> Option<String> {
    let components = epoch_components(epoch)?;
    Some(format!(
        "{:02}-{:02} {:02}:{:02}",
        components.1, components.2, components.3, components.4
    ))
}

pub fn format_epoch_local_datetime_with_offset(epoch: i64) -> Option<String> {
    let base = format_epoch_local_datetime(epoch)?;
    Some(format!("{base} +00:00"))
}

pub fn format_epoch_local(epoch: i64, fmt: &str) -> Option<String> {
    let components = epoch_components(epoch)?;
    Some(format_with_components(fmt, components))
}

pub fn format_until_epoch_compact(target_epoch: i64, now_epoch: i64) -> Option<String> {
    if target_epoch <= 0 || now_epoch <= 0 {
        return None;
    }
    let remaining = target_epoch - now_epoch;
    if remaining <= 0 {
        return Some(format!("{:>2}h {:>2}m", 0, 0));
    }

    if remaining >= 86_400 {
        let days = remaining / 86_400;
        let hours = (remaining % 86_400) / 3_600;
        return Some(format!("{:>2}d {:>2}h", days, hours));
    }

    let hours = remaining / 3_600;
    let minutes = (remaining % 3_600) / 60;
    Some(format!("{:>2}h {:>2}m", hours, minutes))
}

fn remaining_percent(used_percent: f64) -> i64 {
    let remaining = 100.0 - used_percent;
    remaining.round() as i64
}

fn epoch_components(epoch: i64) -> Option<(i64, i64, i64, i64, i64, i64)> {
    if epoch <= 0 {
        return None;
    }
    let days = epoch.div_euclid(86_400);
    let seconds_of_day = epoch.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    Some((year, month, day, hour, minute, second))
}

fn format_with_components(fmt: &str, parts: (i64, i64, i64, i64, i64, i64)) -> String {
    let (year, month, day, hour, minute, second) = parts;
    let mut out = String::with_capacity(fmt.len() + 16);
    let chars: Vec<char> = fmt.chars().collect();
    let mut index = 0usize;

    while index < chars.len() {
        let ch = chars[index];
        if ch != '%' {
            out.push(ch);
            index += 1;
            continue;
        }

        if index + 1 >= chars.len() {
            out.push('%');
            index += 1;
            continue;
        }

        let next = chars[index + 1];
        match next {
            'Y' => out.push_str(&format!("{year:04}")),
            'm' => out.push_str(&format!("{month:02}")),
            'd' => out.push_str(&format!("{day:02}")),
            'H' => out.push_str(&format!("{hour:02}")),
            'M' => out.push_str(&format!("{minute:02}")),
            'S' => out.push_str(&format!("{second:02}")),
            '%' => out.push('%'),
            ':' => {
                if index + 2 < chars.len() && chars[index + 2] == 'z' {
                    out.push_str("+00:00");
                    index += 1;
                } else {
                    out.push('%');
                    out.push(':');
                }
            }
            other => {
                out.push('%');
                out.push(other);
            }
        }
        index += 2;
    }

    out
}

fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 {
        z / 146_097
    } else {
        (z - 146_096) / 146_097
    };
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    (year, month, day)
}
