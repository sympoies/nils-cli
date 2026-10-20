//! Per-command evidence limits, using the existing timed backend capture path.
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::backend::{BackendCall, BackendRunner};
use crate::error::ForgeError;

use super::{parse, schema};

const MAX_REQUESTS: usize = 32;
const MAX_EVIDENCE_BYTES: usize = 16 * 1024 * 1024;
const CALL_TIMEOUT: Duration = Duration::from_secs(30);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(120);

pub(super) struct ReadBudget {
    requests: usize,
    bytes: usize,
    started: Instant,
}

impl Default for ReadBudget {
    fn default() -> Self {
        Self {
            requests: 0,
            bytes: 0,
            started: Instant::now(),
        }
    }
}

impl ReadBudget {
    pub(super) fn read(
        &mut self,
        runner: &impl BackendRunner,
        call: &BackendCall,
    ) -> Result<Value, ForgeError> {
        let remaining = TOTAL_TIMEOUT.saturating_sub(self.started.elapsed());
        if self.requests >= MAX_REQUESTS || remaining.is_zero() {
            return Err(limit());
        }
        self.requests += 1;
        // A finite timeout selects the existing 8 MiB per-stream capture and
        // process-group cleanup path; no backend-wide policy change is needed.
        let output = runner.run_with_timeout(call, Some(CALL_TIMEOUT.min(remaining)))?;
        let bytes = output.stdout.len().saturating_add(output.stderr.len());
        self.bytes = self.bytes.saturating_add(bytes);
        if self.bytes > MAX_EVIDENCE_BYTES {
            return Err(limit());
        }
        parse(&output.stdout)
    }
}

fn limit() -> ForgeError {
    ForgeError::unavailable(
        schema(),
        "security_read_limit",
        "code-scanning read exceeded its request, time or cumulative evidence budget; no complete result emitted",
        None,
    )
}
