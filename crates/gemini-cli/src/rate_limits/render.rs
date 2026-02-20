use std::collections::BTreeMap;

pub struct UsageData {
    pub primary: Window,
    pub secondary: Window,
}

#[derive(Clone)]
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
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    parse_wham_usage(&value).or_else(|| parse_code_assist_usage(&value))
}

fn parse_wham_usage(value: &serde_json::Value) -> Option<UsageData> {
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

fn parse_code_assist_usage(value: &serde_json::Value) -> Option<UsageData> {
    let buckets = value.get("buckets")?.as_array()?;
    let now_epoch = now_epoch_seconds();
    let mut grouped: BTreeMap<i64, f64> = BTreeMap::new();

    for bucket in buckets {
        if let Some(token_type) = bucket.get("tokenType").and_then(|value| value.as_str())
            && !token_type.eq_ignore_ascii_case("REQUESTS")
        {
            continue;
        }

        let remaining_fraction = match bucket
            .get("remainingFraction")
            .and_then(|value| value.as_f64())
        {
            Some(value) => value.clamp(0.0, 1.0),
            None => continue,
        };
        let used_percent = (100.0 - (remaining_fraction * 100.0)).clamp(0.0, 100.0);

        let reset_at = match bucket
            .get("resetTime")
            .and_then(|value| value.as_str())
            .and_then(parse_rfc3339_epoch)
        {
            Some(epoch) if epoch > 0 => epoch,
            _ => continue,
        };

        grouped
            .entry(reset_at)
            // Keep the worst remaining bucket for each reset horizon.
            .and_modify(|existing_used| {
                if used_percent > *existing_used {
                    *existing_used = used_percent;
                }
            })
            .or_insert(used_percent);
    }

    let mut windows: Vec<Window> = grouped
        .iter()
        .map(|(reset_at, used_percent)| {
            let limit_window_seconds = if now_epoch > 0 {
                normalize_window_seconds(reset_at.saturating_sub(now_epoch))
            } else {
                1
            };
            Window {
                limit_window_seconds,
                used_percent: *used_percent,
                reset_at: *reset_at,
            }
        })
        .collect();
    if windows.is_empty() {
        return None;
    }
    windows.sort_by_key(|window| window.reset_at);
    let primary = windows.first()?.clone();
    let secondary = windows.last().cloned().unwrap_or_else(|| primary.clone());

    Some(UsageData { primary, secondary })
}

fn normalize_window_seconds(seconds: i64) -> i64 {
    let clamped = seconds.max(1);
    if clamped >= 3_600 {
        return (clamped / 3_600).max(1) * 3_600;
    }
    if clamped >= 60 {
        return (clamped / 60).max(1) * 60;
    }
    clamped
}

fn now_epoch_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
        .unwrap_or(0)
}

fn parse_rfc3339_epoch(raw: &str) -> Option<i64> {
    let normalized = normalize_iso(raw);
    let (datetime, offset_seconds) = if normalized.ends_with('Z') {
        (&normalized[..normalized.len().saturating_sub(1)], 0i64)
    } else {
        if normalized.len() < 6 {
            return None;
        }
        let tail_index = normalized.len() - 6;
        let sign = normalized.as_bytes().get(tail_index).copied()? as char;
        if sign != '+' && sign != '-' {
            return None;
        }
        if normalized.as_bytes().get(tail_index + 3).copied()? as char != ':' {
            return None;
        }
        let hours = parse_u32(&normalized[tail_index + 1..tail_index + 3])? as i64;
        let minutes = parse_u32(&normalized[tail_index + 4..])? as i64;
        let mut offset = hours * 3_600 + minutes * 60;
        if sign == '-' {
            offset = -offset;
        }
        (&normalized[..tail_index], offset)
    };

    if datetime.len() != 19 {
        return None;
    }
    if datetime.as_bytes().get(4).copied()? as char != '-'
        || datetime.as_bytes().get(7).copied()? as char != '-'
        || datetime.as_bytes().get(10).copied()? as char != 'T'
        || datetime.as_bytes().get(13).copied()? as char != ':'
        || datetime.as_bytes().get(16).copied()? as char != ':'
    {
        return None;
    }

    let year = parse_i64(&datetime[0..4])?;
    let month = parse_u32(&datetime[5..7])? as i64;
    let day = parse_u32(&datetime[8..10])? as i64;
    let hour = parse_u32(&datetime[11..13])? as i64;
    let minute = parse_u32(&datetime[14..16])? as i64;
    let second = parse_u32(&datetime[17..19])? as i64;

    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }

    let days = days_from_civil(year, month, day);
    let local_epoch = days * 86_400 + hour * 3_600 + minute * 60 + second;
    Some(local_epoch - offset_seconds)
}

fn normalize_iso(raw: &str) -> String {
    let mut trimmed = raw
        .split(&['\n', '\r'][..])
        .next()
        .unwrap_or("")
        .to_string();
    if let Some(dot) = trimmed.find('.')
        && trimmed.ends_with('Z')
    {
        trimmed.truncate(dot);
        trimmed.push('Z');
    }
    trimmed
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

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let adjusted_year = year - i64::from(month <= 2);
    let era = if adjusted_year >= 0 {
        adjusted_year / 400
    } else {
        (adjusted_year - 399) / 400
    };
    let year_of_era = adjusted_year - era * 400;
    let month_prime = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn parse_u32(raw: &str) -> Option<u32> {
    raw.parse::<u32>().ok()
}

fn parse_i64(raw: &str) -> Option<i64> {
    raw.parse::<i64>().ok()
}
