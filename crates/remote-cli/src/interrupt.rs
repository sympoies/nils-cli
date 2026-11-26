use std::io;
use std::sync::atomic::{AtomicI32, Ordering};

static PENDING_SIGNAL: AtomicI32 = AtomicI32::new(0);

extern "C" fn remember(signal: libc::c_int) {
    let _ = PENDING_SIGNAL.compare_exchange(0, signal, Ordering::Relaxed, Ordering::Relaxed);
}

pub(super) struct Interrupts {
    previous: Vec<(libc::c_int, libc::sighandler_t)>,
}

impl Interrupts {
    pub(super) fn install() -> io::Result<Self> {
        PENDING_SIGNAL.store(0, Ordering::Relaxed);
        let mut guard = Self {
            previous: Vec::new(),
        };
        for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
            // SAFETY: The handler records only the first signal in a lock-free
            // atomic. Process cleanup runs in the normal wait loop.
            let prior =
                unsafe { libc::signal(signal, remember as *const () as libc::sighandler_t) };
            if prior == libc::SIG_ERR {
                return Err(io::Error::last_os_error());
            }
            guard.previous.push((signal, prior));
        }
        Ok(guard)
    }

    pub(super) fn received(&self) -> Option<i32> {
        let signal = PENDING_SIGNAL.load(Ordering::Relaxed);
        (signal != 0).then_some(signal)
    }
}

impl Drop for Interrupts {
    fn drop(&mut self) {
        for &(signal, prior) in self.previous.iter().rev() {
            // SAFETY: These dispositions were returned by signal() on install.
            unsafe { libc::signal(signal, prior) };
        }
    }
}
