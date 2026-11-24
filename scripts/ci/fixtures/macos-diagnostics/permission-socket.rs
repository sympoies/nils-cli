use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

const IO_BOUND: Duration = Duration::from_secs(2);
const REQUEST: &[u8] = br#"{"request_id":"fixture","call_id":"fixture","decision":"allow"}"#;
const ACK: &[u8] = br#"{"outcome":"delivered_to_hook"}"#;

// F_GETFL is 3 on the two targets of this probe, Darwin and Linux.
unsafe extern "C" {
    fn fcntl(fd: std::ffi::c_int, command: std::ffi::c_int, ...) -> std::ffi::c_int;
}

fn flags(stream: &UnixStream) -> i32 {
    // SAFETY: The stream owns the descriptor and F_GETFL takes no third argument.
    unsafe { fcntl(stream.as_raw_fd(), 3) }
}

struct ReceiveTrace<'a> {
    stream: &'a mut UnixStream,
    started: Instant,
}

impl Read for ReceiveTrace<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let start = self.started.elapsed();
        let status = self.stream.read(buffer);
        eprintln!(
            "{start:?} receiver read ended at {:?}: {status:?}",
            self.started.elapsed()
        );
        status
    }
}

fn main() -> io::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let root = PathBuf::from(args.next().expect("private socket directory argument"));
    let count: usize = args
        .next()
        .map(|arg| arg.to_string_lossy().parse().unwrap())
        .unwrap_or(50);
    for iteration in 0..count {
        let socket = root.join(format!("s{iteration}"));
        let listener = UnixListener::bind(&socket)?;
        listener.set_nonblocking(true)?;
        let mut caller = UnixStream::connect(&socket)?;
        let (mut receiver, _) = listener.accept()?;
        receiver.set_nonblocking(true)?;
        caller.set_read_timeout(Some(IO_BOUND))?;
        let started = Instant::now();
        eprintln!(
            "iteration {iteration}: accepted flags {:#x}",
            flags(&receiver)
        );
        let writer = thread::spawn(move || -> io::Result<Vec<u8>> {
            thread::sleep(Duration::from_millis(50));
            let result = caller.write_all(REQUEST);
            eprintln!("{:?} caller write: {result:?}", started.elapsed());
            result?;
            let result = caller.shutdown(Shutdown::Write);
            eprintln!("{:?} caller half-close: {result:?}", started.elapsed());
            result?;
            let mut received = Vec::new();
            let status = caller.read_to_end(&mut received);
            eprintln!("{:?} caller read to EOF: {status:?}", started.elapsed());
            status?;
            Ok(received)
        });
        receiver.set_nonblocking(false)?;
        receiver.set_read_timeout(Some(IO_BOUND))?;
        receiver.set_write_timeout(Some(IO_BOUND))?;
        eprintln!(
            "{:?} receiver flags after normalization {:#x}",
            started.elapsed(),
            flags(&receiver)
        );
        let mut received = Vec::new();
        let result = ReceiveTrace {
            stream: &mut receiver,
            started,
        }
        .take(4097)
        .read_to_end(&mut received);
        if result.is_ok() && received == REQUEST {
            receiver.write_all(ACK)?;
        }
        drop(receiver);
        let ack = writer.join().expect("writer unexpectedly panicked");
        std::fs::remove_file(socket)?;
        if result.is_err() || received != REQUEST || !matches!(ack.as_deref(), Ok(ACK)) {
            eprintln!(
                "iteration {iteration} failed: receiver read {result:?}, {} request bytes, caller read {ack:?}",
                received.len()
            );
            return Err(io::Error::other("permission socket probe failed"));
        }
    }
    println!("{count}/{count} delayed accepted-stream exchanges passed");
    Ok(())
}
