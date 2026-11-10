use super::{Error, Profile};

/// Classify GitHub's included HTTP response without exposing its body.
pub(super) fn classify(bytes: &[u8], profile: &Profile) -> Option<Error> {
    let text = std::str::from_utf8(bytes).ok()?;
    let (headers, body) = text
        .split_once("\r\n\r\n")
        .or_else(|| text.split_once("\n\n"))?;
    let mut lines = headers.lines();
    let status = lines
        .next()?
        .split_ascii_whitespace()
        .nth(1)?
        .parse::<u16>()
        .ok()?;
    if !matches!(status, 403 | 429) {
        return None;
    }
    let mut remaining_zero = false;
    let mut reset = None;
    let mut retry_after = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("x-ratelimit-remaining") {
            remaining_zero = value.trim() == "0";
        } else if name.eq_ignore_ascii_case("x-ratelimit-reset") {
            reset = value.trim().parse::<i64>().ok();
        } else if name.eq_ignore_ascii_case("retry-after") {
            retry_after = value
                .trim()
                .parse::<i64>()
                .ok()
                .filter(|seconds| *seconds >= 0);
        }
    }
    let message = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value.get("message")?.as_str().map(str::to_ascii_lowercase))
        .unwrap_or_default();
    if !remaining_zero && !message.contains("secondary rate limit") && !message.contains("abuse") {
        return None;
    }
    let timestamp = reset
        .or_else(|| jiff::Timestamp::now().as_second().checked_add(retry_after?))
        .and_then(|seconds| jiff::Timestamp::from_second(seconds).ok())
        .map(|time| time.to_string())
        .unwrap_or_else(|| "an unknown time".to_owned());
    let login = profile
        .expected_login
        .as_deref()
        .or(profile.app_slug.as_deref())
        .unwrap_or("unknown actor");
    Some(Error::with_detail(
        "github_rate_limited",
        format!(
            "GitHub API rate limit exhausted for {login}; resets at {timestamp}; retry after that"
        ),
    ))
}
