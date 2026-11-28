//! Runs a restore action when SIGINT or SIGTERM arrives while a pane is paused.
//!
//! The handler only writes one byte to a pipe, which is async-signal-safe. A
//! helper thread performs the restore, puts the previous dispositions back and
//! re-raises the signal so the process still ends as the sender intended.

use std::os::fd::RawFd;
use std::sync::atomic::{AtomicI32, Ordering};
use std::thread::JoinHandle;

const STOP: u8 = 0;
const SIGNALS: [libc::c_int; 2] = [libc::SIGINT, libc::SIGTERM];
static WAKE_FD: AtomicI32 = AtomicI32::new(-1);

extern "C" fn on_signal(signal: libc::c_int) {
    let fd = WAKE_FD.load(Ordering::Relaxed);
    let byte = signal as u8;
    if fd >= 0 {
        // SAFETY: write(2) is async-signal-safe and reads one initialized byte.
        unsafe { libc::write(fd, (&raw const byte).cast(), 1) };
    }
}

pub(super) struct Watch {
    wake: RawFd,
    previous: [libc::sigaction; 2],
    thread: Option<JoinHandle<()>>,
}

fn restore_dispositions(previous: &[libc::sigaction; 2]) {
    for (signal, action) in SIGNALS.iter().zip(previous) {
        // SAFETY: `action` was returned by sigaction for this signal.
        unsafe { libc::sigaction(*signal, action, std::ptr::null_mut()) };
    }
}

impl Watch {
    pub(super) fn install(on_interrupt: impl FnOnce() + Send + 'static) -> Option<Self> {
        let mut fds = [0 as RawFd; 2];
        // SAFETY: `fds` has room for the two descriptors pipe(2) returns.
        if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
            return None;
        }
        let [read, wake] = fds;
        for fd in fds {
            // SAFETY: both descriptors were just created and are owned here.
            unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
        }
        // SAFETY: an all-zero sigaction is a valid placeholder to be overwritten.
        let mut previous: [libc::sigaction; 2] = unsafe { std::mem::zeroed() };
        WAKE_FD.store(wake, Ordering::Relaxed);
        for (index, signal) in SIGNALS.iter().enumerate() {
            // SAFETY: the action is initialized before it is installed.
            unsafe {
                let mut action: libc::sigaction = std::mem::zeroed();
                action.sa_sigaction = on_signal as extern "C" fn(libc::c_int) as usize;
                libc::sigemptyset(&mut action.sa_mask);
                libc::sigaction(*signal, &action, &mut previous[index]);
            }
        }
        let thread = std::thread::Builder::new()
            .name("pane-input-interrupt".into())
            .spawn(move || {
                let mut byte = STOP;
                let count = loop {
                    // SAFETY: reads at most one byte into `byte`.
                    let count = unsafe { libc::read(read, (&raw mut byte).cast(), 1) };
                    if count >= 0
                        || std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted
                    {
                        break count;
                    }
                };
                // SAFETY: this thread owns the read end.
                unsafe { libc::close(read) };
                if count == 1 && byte != STOP {
                    on_interrupt();
                    restore_dispositions(&previous);
                    // SAFETY: re-raises the signal the process was asked to die from.
                    unsafe { libc::raise(libc::c_int::from(byte)) };
                }
            });
        match thread {
            Ok(thread) => Some(Self {
                wake,
                previous,
                thread: Some(thread),
            }),
            Err(_) => {
                restore_dispositions(&previous);
                WAKE_FD.store(-1, Ordering::Relaxed);
                // SAFETY: the thread never started, so both ends are still owned here.
                unsafe {
                    libc::close(read);
                    libc::close(wake);
                }
                None
            }
        }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        restore_dispositions(&self.previous);
        WAKE_FD.store(-1, Ordering::Relaxed);
        let stop = STOP;
        // SAFETY: writes one byte to the pipe this value owns, then closes it.
        unsafe {
            libc::write(self.wake, (&raw const stop).cast(), 1);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        // SAFETY: the helper thread has finished; only the write end remains.
        unsafe { libc::close(self.wake) };
    }
}
