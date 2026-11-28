//! The observational source lock that resume admission holds through launch.
use super::*;
use std::time::Instant;

/// How long admission waits for a source session's record lock before it calls
/// the source live. Housekeeping that shares this lock (the serve loops, boot
/// recovery) bounds its own wait for it at the same 250 ms and holds it only
/// for a short critical section, so a holder that outlasts the bound is a
/// genuine in-flight writer, such as another resume or restart, and still
/// refuses with `resume-source-live`.
pub(super) const SOURCE_LOCK_WAIT: Duration = Duration::from_millis(250);
const SOURCE_LOCK_POLL: Duration = Duration::from_millis(5);

pub(super) fn lock_source(context: &CliContext, id: &str) -> Result<fs::File, CliError> {
    lock_source_within(context, id, SOURCE_LOCK_WAIT)
}

fn lock_source_within(
    context: &CliContext,
    id: &str,
    wait: Duration,
) -> Result<fs::File, CliError> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(
            context
                .state_dir
                .join(SESSION_LOCKS_DIR)
                .join(format!("{id}.lock")),
        )
        .map_err(|_| refusal("resume-source-unknown"))?;
    let deadline = Instant::now() + wait;
    loop {
        // SAFETY: the descriptor remains owned by Admission for the whole launch.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(file);
        }
        if io::Error::last_os_error().kind() != io::ErrorKind::WouldBlock {
            return Err(refusal("resume-source-unknown"));
        }
        if Instant::now() >= deadline {
            return Err(refusal("resume-source-live"));
        }
        thread::sleep(SOURCE_LOCK_POLL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::sync::mpsc;

    /// Holds the source lock on its own descriptor until told to release it.
    struct Holder {
        release: mpsc::Sender<()>,
        thread: thread::JoinHandle<()>,
    }

    impl Holder {
        fn take(context: &CliContext, id: &str) -> Self {
            let (held_tx, held_rx) = mpsc::channel();
            let (release, release_rx) = mpsc::channel::<()>();
            let context = context.clone();
            let id = id.to_string();
            let thread = thread::spawn(move || {
                let file = lock_source_within(&context, &id, Duration::ZERO).expect("holder lock");
                held_tx.send(()).unwrap();
                let _ = release_rx.recv();
                drop(file);
            });
            held_rx.recv().expect("holder took the lock");
            Self { release, thread }
        }

        fn release(self) {
            let _ = self.release.send(());
            self.thread.join().unwrap();
        }
    }

    fn fixture() -> (tempfile::TempDir, CliContext) {
        let tmp = tempfile::TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(SESSION_LOCKS_DIR)).unwrap();
        fs::write(tmp.path().join(SESSION_LOCKS_DIR).join("source.lock"), "").unwrap();
        let context = CliContext {
            state_dir: tmp.path().to_path_buf(),
            host: None,
        };
        (tmp, context)
    }

    #[test]
    fn a_holder_that_releases_within_the_wait_does_not_read_as_live() {
        let (_tmp, context) = fixture();
        let holder = Holder::take(&context, "source");
        let releaser = thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            holder.release();
        });
        // The wait is far wider than the release, so only the release order
        // decides the outcome, never how slowly the host schedules either side.
        let locked = lock_source_within(&context, "source", Duration::from_secs(30));
        assert!(
            locked.is_ok(),
            "lock must be acquired after the holder released: {:?}",
            locked.as_ref().err().map(CliError::code)
        );
        releaser.join().unwrap();
    }

    #[test]
    fn a_holder_that_outlasts_the_wait_is_still_live() {
        let (_tmp, context) = fixture();
        let holder = Holder::take(&context, "source");
        let error = lock_source_within(&context, "source", Duration::from_millis(30))
            .expect_err("a held source lock must refuse");
        assert_eq!(error.code(), "resume-source-live");
        holder.release();
        lock_source_within(&context, "source", Duration::ZERO).expect("free after release");
    }
}
