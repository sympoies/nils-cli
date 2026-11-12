//! Run independent fixture cases in small parallel batches.
use std::fmt::Debug;

/// Each case builds its own `Fixture` (temp directory and session state) and
/// runs the real binary, so cases do not share anything. A small bound keeps the
/// load close to what nextest already runs in parallel.
pub(crate) const CONCURRENT_CASES: usize = 4;

/// Run `check` for every case, at most `CONCURRENT_CASES` at a time. Every case
/// in a batch finishes before the next batch starts, and the failure names each
/// failing case; the failing assertion itself prints from its own thread.
pub(crate) fn for_each_case<T: Debug + Sync>(cases: &[T], check: impl Fn(&T) + Sync) {
    let check = &check;
    for batch in cases.chunks(CONCURRENT_CASES) {
        std::thread::scope(|scope| {
            let runs: Vec<_> = batch
                .iter()
                .map(|case| scope.spawn(move || check(case)))
                .collect();
            let failed: Vec<_> = runs
                .into_iter()
                .zip(batch)
                .filter_map(|(run, case)| run.join().err().map(|_| format!("{case:?}")))
                .collect();
            assert!(failed.is_empty(), "cases failed: {failed:#?}");
        });
    }
}
