//! Page-scoped History attestations. Holding the store lock fences publication.
use super::*;

pub(crate) struct Reader {
    ledger: Ledger,
    history: String,
    live: HashMap<u64, Option<Confirmed>>,
    deadline: Instant,
}

impl Reader {
    pub(crate) fn open(state: &Path, history: &str, deadline: Instant) -> io::Result<Option<Self>> {
        let Some(mut ledger) = Ledger::open(state, false, deadline)? else {
            return Ok(None);
        };
        let scan = ledger.scan(deadline)?;
        let mut live = HashMap::new();
        for terminal in scan.terminals.values() {
            let claim = &scan.claims.get(&terminal.op).ok_or_else(invalid)?.0;
            if claim.history_id != history {
                continue;
            }
            if let Some(proof) = terminal.confirmed(claim) {
                use std::collections::hash_map::Entry;
                match live.entry(proof.row_offset) {
                    Entry::Vacant(e) => {
                        e.insert(Some(proof));
                    }
                    Entry::Occupied(mut e) => {
                        e.insert(None);
                    }
                }
            }
        }
        Ok(Some(Self {
            ledger,
            history: history.into(),
            live,
            deadline,
        }))
    }

    /// Outer None means unbound; inner None is an ambiguous physical row.
    pub(crate) fn proof(&self, offset: u64) -> io::Result<Option<Option<Confirmed>>> {
        lifecycle::check_time(self.deadline)?;
        let key = lifecycle::checkpoint::proof_key(&self.history, offset);
        let archived = lifecycle::checkpoint::get(&self.ledger, &key, self.deadline)?
            .map(|value| {
                if value.is_empty() {
                    Ok(None)
                } else {
                    let proof: Confirmed = serde_json::from_str(&value)?;
                    if !proof.claim.valid()
                        || proof.claim.history_id != self.history
                        || proof.row_offset != offset
                        || uuid::Uuid::parse_str(&proof.native_uuid).is_err()
                        || !hash_valid(&proof.row_payload_sha256)
                    {
                        return Err(invalid());
                    }
                    Ok(Some(proof))
                }
            })
            .transpose()?;
        match (self.live.get(&offset), archived) {
            (None, archived) => Ok(archived),
            (Some(live), None) => Ok(Some(live.clone())),
            (Some(Some(live)), Some(Some(archived)))
                if serde_json::to_vec(live)? == serde_json::to_vec(&archived)? =>
            {
                Ok(Some(Some(archived)))
            }
            _ => Ok(Some(None)),
        }
    }
}
