//! Receipt-only native notice observations; ordinary human prompt rules stay intact.
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::send::claude_notice::{Binding, PLUGIN, sha256, version};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Stamp {
    pub(crate) device: u64,
    pub(crate) inode: u64,
    pub(crate) eof: u64,
    pub(crate) continuity_sha256: String,
}

pub(crate) struct Baseline {
    pub(crate) stamp: Stamp,
    path: PathBuf,
    file: File,
}

#[derive(Clone, Debug)]
pub(crate) struct Row {
    pub(crate) native_uuid: String,
    pub(crate) text: String,
    pub(crate) timestamp: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct Proof {
    pub(crate) row: Row,
    pub(crate) row_offset: u64,
    pub(crate) row_payload_sha256: String,
}

/// Decode only the pinned external native row shape. This confers no authority
/// without the private producer claim and exact physical-row receipt binding.
pub(crate) fn row(value: &Value, binding: &Binding) -> Option<Row> {
    if binding.provider != "claude"
        || !version::supported(&binding.provider_version)
        || value.get("type")?.as_str()? != "user"
        || value.get("sessionId")?.as_str()? != binding.provider_session_id
        || value.get("version")?.as_str()? != binding.provider_version
        || value.get("promptSource")?.as_str()? != "system"
        || value.get("isSidechain")?.as_bool()?
    {
        return None;
    }
    for field in ["isMeta", "isCompactSummary", "isVisibleInTranscriptOnly"] {
        if value
            .get(field)
            .is_some_and(|flag| flag != &Value::Bool(false))
        {
            return None;
        }
    }
    let origin = value.get("origin")?;
    if origin.as_object()?.len() != 3
        || origin.get("kind")?.as_str()? != "plugin"
        || origin.get("name")?.as_str()? != PLUGIN
        || !origin.get("asUser")?.as_bool()?
    {
        return None;
    }
    let native_uuid = value.get("uuid")?.as_str()?;
    uuid::Uuid::parse_str(native_uuid).ok()?;
    let message = value.get("message")?;
    if message.get("role")?.as_str()? != "user" {
        return None;
    }
    let content = message.get("content")?;
    let text = if let Some(text) = content.as_str() {
        text
    } else {
        let blocks = content.as_array()?;
        if blocks.len() != 1 || blocks[0].get("type")?.as_str()? != "text" {
            return None;
        }
        blocks[0].get("text")?.as_str()?
    };
    if text.len() > super::MAX_PROVIDER_PROMPT_BYTES
        || text.contains('\r')
        || crate::notice::parse_notice(text).is_none()
    {
        return None;
    }
    let timestamp = value
        .get("timestamp")
        .and_then(Value::as_str)
        .filter(|timestamp| timestamp.len() <= 64)
        .map(str::to_string);
    Some(Row {
        native_uuid: native_uuid.into(),
        text: text.into(),
        timestamp,
    })
}

/// JSONL framing is outside the JSON payload, never part of the logical text.
pub(crate) fn payload(line: &[u8]) -> &[u8] {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    line.strip_suffix(b"\r").unwrap_or(line)
}

impl Baseline {
    pub(crate) fn capture(record: &crate::SessionRecord) -> Option<Self> {
        let source = super::resolve_provider_prompt_source(record)?;
        if source.provider != super::ProviderKind::Claude {
            return None;
        }
        let path = source.validated_path()?.to_path_buf();
        let (mut file, metadata) = super::open_regular_file(&path).ok()?;
        let eof = metadata.len();
        if eof > 0 {
            file.seek(SeekFrom::Start(eof - 1)).ok()?;
            let mut end = [0];
            file.read_exact(&mut end).ok()?;
            if end != *b"\n" {
                return None;
            }
        }
        let continuity = super::read_continuity(&mut file, eof).ok()?;
        if !super::opened_file_still_current(&path, super::file_identity(&metadata)) {
            return None;
        }
        Some(Self {
            stamp: Stamp {
                device: metadata.dev(),
                inode: metadata.ino(),
                eof,
                continuity_sha256: sha256(&continuity),
            },
            path,
            file,
        })
    }

    fn snapshot(
        &mut self,
        binding: &Binding,
        expected: &str,
        deadline: Instant,
    ) -> Result<Option<Proof>, ()> {
        if Instant::now() >= deadline {
            return Err(());
        }
        let metadata = self.file.metadata().map_err(|_| ())?;
        if !super::opened_file_still_current(&self.path, super::file_identity(&metadata))
            || (metadata.dev(), metadata.ino()) != (self.stamp.device, self.stamp.inode)
            || metadata.len() < self.stamp.eof
            || metadata.len() - self.stamp.eof > 1024 * 1024
        {
            return Err(());
        }
        let continuity = super::read_continuity(&mut self.file, self.stamp.eof).map_err(|_| ())?;
        if sha256(&continuity) != self.stamp.continuity_sha256 {
            return Err(());
        }
        self.file
            .seek(SeekFrom::Start(self.stamp.eof))
            .map_err(|_| ())?;
        let mut bytes = vec![0; (metadata.len() - self.stamp.eof) as usize];
        self.file.read_exact(&mut bytes).map_err(|_| ())?;
        if bytes.last().is_some_and(|byte| *byte != b'\n') {
            return Ok(None);
        }
        let mut proof = None;
        let mut offset = self.stamp.eof;
        let mut uuids = std::collections::HashSet::new();
        for line in bytes.split_inclusive(|byte| *byte == b'\n') {
            if Instant::now() >= deadline || line.len() > super::MAX_PROVIDER_LINE_BYTES {
                return Err(());
            }
            let current = offset;
            offset += line.len() as u64;
            let payload = payload(line);
            let value: Value = serde_json::from_slice(payload).map_err(|_| ())?;
            if let Some(row) = row(&value, binding) {
                if !uuids.insert(row.native_uuid.clone()) {
                    return Err(());
                }
                if row.text == expected {
                    if proof.is_some() {
                        return Err(());
                    }
                    proof = Some(Proof {
                        row,
                        row_offset: current,
                        row_payload_sha256: sha256(payload),
                    });
                }
            }
        }
        Ok(proof)
    }

    pub(crate) fn confirm(
        &mut self,
        binding: &Binding,
        expected: &str,
        deadline: Instant,
    ) -> Option<Proof> {
        loop {
            match self.snapshot(binding, expected, deadline) {
                Ok(Some(proof)) => return Some(proof),
                Ok(None) if Instant::now() < deadline => std::thread::sleep(
                    Duration::from_millis(10)
                        .min(deadline.saturating_duration_since(Instant::now())),
                ),
                _ => return None,
            }
        }
    }
}
