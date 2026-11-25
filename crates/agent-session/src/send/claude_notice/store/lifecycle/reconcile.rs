//! Settle abandoned owners without creating submission evidence.
use super::*;
use crate::CoordinationRuntimeStatus;
use crate::send::claude_notice::Launch;

// A live sender has at most 30 seconds to record its exact positive proof.
const EXPIRY_MILLIS: i64 = 120_000;

pub(super) fn settle(
    ledger: &mut Ledger,
    scan: &mut Scan,
    retirement_context: Option<&CliContext>,
    first_seen: &mut BTreeMap<String, String>,
    now: jiff::Timestamp,
    deadline: Instant,
) -> io::Result<()> {
    let mut retired = HashMap::new();
    let mut outcomes = Vec::new();
    let mut lines = Vec::new();
    for (op, (claim, hash)) in &scan.claims {
        check_time(deadline)?;
        if scan.terminals.contains_key(op) {
            continue;
        }
        let launch = &claim.binding.launch;
        let elapsed = age_millis(op, first_seen, now)?;
        let reason = if elapsed >= EXPIRY_MILLIS {
            Reason::ClaimExpired
        } else if retirement_context.is_some_and(|context| {
            *retired
                .entry((&launch.session_id, &launch.incarnation, launch.generation))
                .or_insert_with(|| runtime_retired(context, launch))
        }) {
            Reason::RuntimeRetired
        } else {
            continue;
        };
        let terminal = Terminal::outcome(claim, hash, None, reason, Duration::ZERO);
        lines.extend(encode_line("terminal", &terminal)?);
        outcomes.push((op.clone(), terminal));
    }
    if !lines.is_empty() {
        // Persist terminal records together before the existing rotation can
        // seal them and remove them from the current live segment.
        ledger.append_lines(&lines, deadline)?;
        scan.terminals.extend(outcomes);
    }
    Ok(())
}

fn age_millis(
    op: &str,
    first_seen: &mut BTreeMap<String, String>,
    now: jiff::Timestamp,
) -> io::Result<i64> {
    let uuid = uuid::Uuid::parse_str(op).map_err(|_| invalid())?;
    if uuid.get_version() == Some(uuid::Version::SortRand)
        && let Some(timestamp) = uuid.get_timestamp()
    {
        let (seconds, nanos) = timestamp.to_unix();
        let millis = seconds as i64 * 1_000 + i64::from(nanos / 1_000_000);
        if millis <= now.as_millisecond() {
            return Ok(now.as_millisecond() - millis);
        }
    }
    // Older operation IDs have no claim time. Preserve the first observation
    // in the existing manifest rather than guessing when delivery began.
    let observed = first_seen
        .entry(op.to_owned())
        .or_insert_with(|| now.to_string())
        .parse::<jiff::Timestamp>()
        .map_err(|_| invalid())?;
    Ok(now
        .as_millisecond()
        .saturating_sub(observed.as_millisecond()))
}

fn runtime_retired(context: &CliContext, launch: &Launch) -> bool {
    let record = match crate::load_session_record(context, &launch.session_id) {
        Ok(record) => record,
        Err(error) => return error.code() == "session-not-found",
    };
    record.id != launch.session_id
        || record.runtime.as_ref().is_none_or(|runtime| {
            runtime.launch_id != launch.incarnation || runtime.generation != launch.generation
        })
        || crate::coordination_runtime_evidence(context, &record)
            .is_ok_and(|evidence| evidence.status == CoordinationRuntimeStatus::Stopped)
}
