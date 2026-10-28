//! Account-shared snapshot projection and bounded table diagnostics.
use super::*;
use crate::prompt_segment::client;

pub(super) fn import_shared_cache(target: &Path, name: &str) {
    if let Ok(login) = read_target_login(target)
        && let Some(body) = client::cached_usage(&login.access_token)
        && let Fetch::Windows { values, .. } = parse_usage_body(&body.body)
    {
        write_cache(
            name,
            &values,
            Some(&crate::prompt_segment::token_key(&login.access_token)),
            body.fetched_at,
        );
    }
}

pub(super) fn failure_note(target: &Path, reason: Option<ProviderUsageReason>) -> Option<String> {
    let reason = reason?;
    if reason != ProviderUsageReason::RateLimited {
        return Some(reason.as_str().to_string());
    }
    let login = read_target_login(target).ok()?;
    let mut note = client::retry_at(&login.access_token)
        .and_then(|epoch| i64::try_from(epoch).ok())
        .and_then(format_local_with_offset)
        .map(|until| format!("rate limited until {until}"))
        .unwrap_or_else(|| "rate limited".to_string());
    let owner = crate::prompt_segment::token_key(&login.access_token);
    let age = cache::owned_age(&target_name(target), &owner, now_epoch()).or_else(|| {
        client::fetched_at(&login.access_token)
            .map(|epoch| now_epoch().saturating_sub(epoch).max(0))
    });
    if let Some(age) = age {
        note.push_str(&format!("; cache age={age}s"));
    }
    Some(note)
}

pub(super) fn cached_one_line(target: &Path, name: &str) -> OneLineFetch {
    let mut result = super::cached_one_line(name);
    if read_target_login(target).is_ok_and(|login| client::backoff_active(&login.access_token)) {
        result.note = failure_note(target, Some(ProviderUsageReason::RateLimited));
        result.stale = true;
    }
    result
}
