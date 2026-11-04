//! Strict record validation shared by the live segment and sealed archive readers.
use super::*;

pub(super) fn scan(
    bytes: &[u8],
    deadline: Instant,
    header: Option<&lifecycle::Header>,
) -> io::Result<Scan> {
    let cap = if header.is_some() {
        lifecycle::MAX_SEGMENT_BYTES
    } else {
        MAX_BYTES
    };
    if bytes.len() > cap || bytes.last().is_some_and(|byte| *byte != b'\n') {
        return Err(invalid());
    }
    let mut lines = bytes.split_inclusive(|byte| *byte == b'\n');
    if let Some(expected) = header {
        let line = lines.next().ok_or_else(invalid)?;
        if line.len() > MAX_RECORD {
            return Err(invalid());
        }
        let observed: lifecycle::Header = serde_json::from_slice(line)?;
        if &observed != expected {
            return Err(invalid());
        }
    }
    let mut claims = HashMap::new();
    let mut terminals = HashMap::new();
    for (index, line) in lines.enumerate() {
        if index >= MAX_CLAIMS * 2 || line.len() > MAX_RECORD || Instant::now() >= deadline {
            return Err(invalid());
        }
        let payload = &line[..line.len() - 1];
        let mut value: serde_json::Value = serde_json::from_slice(payload)?;
        let kind = value
            .as_object_mut()
            .ok_or_else(invalid)?
            .remove("kind")
            .ok_or_else(invalid)?;
        match kind.as_str() {
            Some("claim") => {
                let claim: Claim = serde_json::from_value(value)?;
                if !claim.valid() || claims.len() >= MAX_CLAIMS || claims.contains_key(&claim.op) {
                    return Err(invalid());
                }
                claims.insert(claim.op.clone(), (claim, sha256(payload)));
            }
            Some("terminal") => {
                let terminal: Terminal = serde_json::from_value(value)?;
                let (claim, hash) = claims.get(&terminal.op).ok_or_else(invalid)?;
                if terminal.claim_sha256 != *hash
                    || !terminal.valid(claim)
                    || terminals.insert(terminal.op.clone(), terminal).is_some()
                {
                    return Err(invalid());
                }
            }
            _ => return Err(invalid()),
        }
    }
    Ok(Scan { claims, terminals })
}

pub(super) fn merge(target: &mut Scan, source: Scan) -> io::Result<()> {
    for (op, claim) in source.claims {
        if let Some(existing) = target.claims.get(&op) {
            if existing.1 != claim.1 {
                return Err(invalid());
            }
        } else {
            target.claims.insert(op, claim);
        }
    }
    for (op, terminal) in source.terminals {
        if let Some(existing) = target.terminals.get(&op) {
            if serde_json::to_value(existing)? != serde_json::to_value(&terminal)? {
                return Err(invalid());
            }
        } else {
            target.terminals.insert(op, terminal);
        }
    }
    Ok(())
}
