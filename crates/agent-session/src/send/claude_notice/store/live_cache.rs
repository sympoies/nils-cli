//! Share a verified live segment across History readers while the store is locked.
use super::*;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FileStamp {
    device: u64,
    inode: u64,
    len: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

impl FileStamp {
    pub(crate) fn from_metadata(metadata: &fs::Metadata) -> Option<Self> {
        metadata.is_file().then(|| Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            len: metadata.len(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        })
    }
}

type HistoryProofs = HashMap<u64, Option<Confirmed>>;
pub(super) type LiveProofs = HashMap<String, HistoryProofs>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct IndexStamp {
    pub(super) file: FileStamp,
    pub(super) committed: u64,
    pub(super) root: Option<pages::Page>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StoreStamp {
    file: FileStamp,
    header: Option<lifecycle::Header>,
    index: Option<IndexStamp>,
}

impl StoreStamp {
    pub(super) fn read(ledger: &Ledger) -> io::Result<Self> {
        Ok(Self {
            file: FileStamp::from_metadata(&ledger.file.metadata()?).ok_or_else(invalid)?,
            header: lifecycle::expected_header(ledger),
            index: lifecycle::checkpoint::index_stamp(ledger)?,
        })
    }
}

#[derive(Debug, Default)]
pub(crate) struct LiveCache {
    current: Mutex<Option<(StoreStamp, Arc<LiveProofs>)>>,
}

impl LiveCache {
    pub(super) fn get(
        &self,
        ledger: &mut Ledger,
        stamp: &StoreStamp,
        deadline: Instant,
    ) -> io::Result<Arc<LiveProofs>> {
        let mut current = self
            .current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        lifecycle::check_time(deadline)?;
        if let Some((previous, proofs)) = current.as_ref()
            && previous == stamp
        {
            return Ok(Arc::clone(proofs));
        }

        let scan = ledger.scan(deadline)?;
        let mut proofs = LiveProofs::new();
        for terminal in scan.terminals.values() {
            let claim = &scan.claims.get(&terminal.op).ok_or_else(invalid)?.0;
            if let Some(proof) = terminal.confirmed(claim) {
                let rows = proofs.entry(claim.history_id.clone()).or_default();
                match rows.entry(proof.row_offset) {
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        entry.insert(Some(proof));
                    }
                    std::collections::hash_map::Entry::Occupied(mut entry) => {
                        entry.insert(None);
                    }
                }
            }
        }
        lifecycle::check_time(deadline)?;
        if StoreStamp::read(ledger)? != *stamp {
            return Err(invalid());
        }
        let proofs = Arc::new(proofs);
        *current = Some((stamp.clone(), Arc::clone(&proofs)));
        Ok(proofs)
    }
}
