use std::cell::Cell;

thread_local! {
    static EPOCH: Cell<Option<i64>> = const { Cell::new(None) };
}

pub(super) fn epoch() -> Option<i64> {
    EPOCH.get()
}

/// Keep synchronous fixture operations at one epoch, even while descheduled.
pub(super) fn freeze() -> impl Drop {
    struct Restore(Option<i64>);
    impl Drop for Restore {
        fn drop(&mut self) {
            EPOCH.set(self.0);
        }
    }

    Restore(EPOCH.replace(Some(super::now_epoch())))
}
