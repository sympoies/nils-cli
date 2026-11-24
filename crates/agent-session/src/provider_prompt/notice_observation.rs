//! Keep the scanned boundary for an unresolved tmux Codex notice attempt.
use super::*;
use crate::send::claude_notice::store::FileStamp;
use jiff::Timestamp;
use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

const MAX_ATTEMPTS: usize = 64;

type AttemptKey = (PathBuf, String, Timestamp, String);

struct Observation {
    stamp: FileStamp,
    identity: FileIdentity,
    length: u64,
    complete_offset: u64,
    continuity: Vec<u8>,
    matched: bool,
}

pub(crate) fn notice_observed_after(
    source: &ProviderPromptSource,
    prompt: &str,
    attempted_at: &Timestamp,
) -> bool {
    match source.provider {
        ProviderKind::Codex => observe_codex(source, prompt, attempted_at).unwrap_or(false),
        ProviderKind::Claude => {
            super::prompt_observed_after(source, prompt, attempted_at) == Some(true)
        }
    }
}

fn observe_codex(
    source: &ProviderPromptSource,
    prompt: &str,
    attempted_at: &Timestamp,
) -> io::Result<bool> {
    static CACHE: OnceLock<Mutex<BTreeMap<AttemptKey, Observation>>> = OnceLock::new();
    let key = (
        source.path.clone(),
        source.session_id.clone(),
        *attempted_at,
        crate::send::claude_notice::sha256(prompt.as_bytes()),
    );
    let mut cache = CACHE
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    let (mut file, metadata) = open_regular_file(&source.path)?;
    let Some(stamp) = FileStamp::from_metadata(&metadata) else {
        return Ok(false);
    };
    let previous = cache.get(&key);
    if let Some(previous) = previous
        && previous.stamp == stamp
    {
        return Ok(previous.matched);
    }
    if nils_provider_resume::read_codex_resumable_session_meta_from_file(&mut file)
        .is_none_or(|header| header.session_id != source.session_id)
    {
        return Ok(false);
    }

    let identity = file_identity(&metadata);
    let length = metadata.len();
    let window_start = length.saturating_sub(MAX_LAST_PROMPT_RECOVERY_BYTES as u64);
    // Resume only a negative scan on an append to the same rollout. Revisit
    // incomplete JSON on growth and rebuild the bounded scan on a discontinuity.
    let start = match previous {
        Some(previous)
            if !previous.matched
                && identity == previous.identity
                && length > previous.length
                && previous.complete_offset >= window_start
                && continuity_matches(
                    &mut file,
                    previous.complete_offset,
                    &previous.continuity,
                )? =>
        {
            previous.complete_offset
        }
        _ => window_start,
    };
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    (&mut file).take(length - start).read_to_end(&mut bytes)?;
    let matched = bytes.split(|byte| *byte == b'\n').any(|line| {
        std::str::from_utf8(line)
            .ok()
            .and_then(parse_codex_prompt)
            .is_some_and(|parsed| {
                parsed.prompt == prompt
                    && parsed
                        .submitted_at
                        .as_deref()
                        .and_then(|value| value.parse::<Timestamp>().ok())
                        .is_some_and(|submitted| submitted > *attempted_at)
            })
    });
    let complete_offset = start
        + bytes
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |end| end as u64 + 1);
    let continuity = read_continuity(&mut file, complete_offset)?;
    if fs::symlink_metadata(&source.path)
        .ok()
        .and_then(|metadata| FileStamp::from_metadata(&metadata))
        .as_ref()
        != Some(&stamp)
    {
        return Ok(false);
    }

    if previous.is_none() && cache.len() >= MAX_ATTEMPTS {
        cache.pop_first();
    }
    cache.insert(
        key,
        Observation {
            stamp,
            identity,
            length,
            complete_offset,
            continuity,
            matched,
        },
    );
    Ok(matched)
}
