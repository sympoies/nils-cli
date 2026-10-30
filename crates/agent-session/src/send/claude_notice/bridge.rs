//! One caller-owned HTTP exchange over an owned private Unix socket.
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{Value, json};

use super::{Binding, runtime};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Authorize {
    pub(super) schema_version: String,
    pub(super) op: String,
    pub(super) binding: Binding,
    pub(super) nonce: String,
    pub(super) terminal: bool,
    pub(super) box_empty: bool,
    pub(super) cursor: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Origin {
    pub(super) kind: String,
    pub(super) name: String,
    pub(super) as_user: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Report {
    pub(super) schema_version: String,
    pub(super) op: String,
    pub(super) binding: Binding,
    pub(super) nonce: String,
    pub(super) outcome: String,
    pub(super) after_empty: bool,
    pub(super) returned_text: Option<String>,
    pub(super) origin: Option<Origin>,
}

pub(super) struct Listener {
    listener: UnixListener,
    socket: PathBuf,
    identity: (u64, u64),
    dir: PathBuf,
    op: String,
    _gate: File,
}

pub(super) struct Connection {
    stream: UnixStream,
    pub(super) path: String,
    pub(super) value: Value,
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "native notice protocol unavailable",
    )
}

pub(super) fn remaining(deadline: Instant) -> io::Result<Duration> {
    let duration = deadline.saturating_duration_since(Instant::now());
    if duration.is_zero() {
        return Err(io::ErrorKind::TimedOut.into());
    }
    Ok(duration)
}

fn same_uid(stream: &UnixStream) -> bool {
    #[cfg(target_os = "linux")]
    {
        let mut credentials: libc::ucred = libc::ucred {
            pid: 0,
            uid: 0,
            gid: 0,
        };
        let mut size = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        // SAFETY: the stream owns fd; pointers identify initialized storage of size.
        let result = unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut credentials as *mut libc::ucred).cast(),
                &mut size,
            )
        };
        // SAFETY: geteuid has no preconditions.
        result == 0 && credentials.uid == unsafe { libc::geteuid() }
    }
    #[cfg(target_os = "macos")]
    {
        let mut uid = 0;
        let mut gid = 0;
        // SAFETY: descriptor is live; uid and gid are initialized output storage.
        let result = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) };
        // SAFETY: geteuid has no preconditions.
        result == 0 && uid == unsafe { libc::geteuid() }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = stream;
        false
    }
}

fn read_exact(stream: &mut UnixStream, mut output: &mut [u8], deadline: Instant) -> io::Result<()> {
    while !output.is_empty() {
        stream.set_read_timeout(Some(remaining(deadline)?))?;
        let count = stream.read(output)?;
        if count == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        output = &mut output[count..];
    }
    remaining(deadline)?;
    Ok(())
}

fn publish(dir: &Path, value: &Value) -> io::Result<()> {
    runtime::private_read(&dir.join("offer.json"), 4096).map_err(|_| invalid())?;
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > 4096 {
        return Err(invalid());
    }
    let temporary = dir.join(format!("offer-{}.tmp", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&temporary)?;
    let result = file
        .write_all(&bytes)
        .and_then(|()| fs::rename(&temporary, dir.join("offer.json")));
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

impl Listener {
    pub(super) fn open(
        dir: PathBuf,
        boot: &runtime::Bootstrap,
        binding: &Binding,
        op: String,
    ) -> io::Result<Self> {
        let gate = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(dir.join("operation.lock"))?;
        let metadata = gate.metadata()?;
        // SAFETY: geteuid has no preconditions.
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o777 != 0o600
        {
            return Err(invalid());
        }
        // SAFETY: gate owns the descriptor until the listener is dropped.
        if unsafe { libc::flock(gate.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(io::Error::last_os_error());
        }
        // Never overwrite or guess ownership of a preexisting endpoint.
        let listener = UnixListener::bind(&boot.socket_path)?;
        let metadata = fs::symlink_metadata(&boot.socket_path)?;
        let owner = Self {
            listener,
            socket: boot.socket_path.clone(),
            identity: (metadata.dev(), metadata.ino()),
            dir,
            op,
            _gate: gate,
        };
        fs::set_permissions(&boot.socket_path, fs::Permissions::from_mode(0o600))?;
        owner.listener.set_nonblocking(true)?;
        publish(
            &owner.dir,
            &json!({"schema_version":runtime::OFFER_SCHEMA,"op":owner.op,
            "binding":binding,"socket_path":boot.socket_path,"nonce":boot.nonce}),
        )?;
        Ok(owner)
    }

    pub(super) fn receive(&self, deadline: Instant) -> io::Result<Connection> {
        let mut stream = loop {
            remaining(deadline)?;
            match self.listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(2).min(remaining(deadline)?))
                }
                Err(error) => return Err(error),
            }
        };
        if !same_uid(&stream) {
            return Err(invalid());
        }
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") {
            if header.len() >= 8192 {
                return Err(invalid());
            }
            let mut byte = [0];
            read_exact(&mut stream, &mut byte, deadline)?;
            header.push(byte[0]);
        }
        let header = std::str::from_utf8(&header).map_err(|_| invalid())?;
        let mut lines = header.split("\r\n");
        let request = lines
            .next()
            .ok_or_else(invalid)?
            .split(' ')
            .collect::<Vec<_>>();
        if request.len() != 3
            || request[0] != "POST"
            || request[2] != "HTTP/1.1"
            || !matches!(request[1], "/authorize" | "/result")
        {
            return Err(invalid());
        }
        let mut size = None;
        for line in lines.filter(|line| !line.is_empty()) {
            let (name, value) = line.split_once(':').ok_or_else(invalid)?;
            if name.eq_ignore_ascii_case("transfer-encoding") {
                return Err(invalid());
            }
            if name.eq_ignore_ascii_case("content-length") {
                if size.is_some() {
                    return Err(invalid());
                }
                size = Some(value.trim().parse::<usize>().map_err(|_| invalid())?);
            }
        }
        let size = size.ok_or_else(invalid)?;
        let cap = if request[1] == "/authorize" {
            4096
        } else {
            128 * 1024
        };
        if size == 0 || size > cap {
            return Err(invalid());
        }
        let mut body = vec![0; size];
        read_exact(&mut stream, &mut body, deadline)?;
        Ok(Connection {
            stream,
            path: request[1].to_string(),
            value: serde_json::from_slice(&body)?,
        })
    }
}

impl Connection {
    pub(super) fn parse<T: serde::de::DeserializeOwned>(&self) -> io::Result<T> {
        serde_json::from_value(self.value.clone()).map_err(|_| invalid())
    }

    pub(super) fn respond(mut self, value: &Value, deadline: Instant) -> io::Result<()> {
        let body = serde_json::to_vec(value)?;
        if body.len() > 128 * 1024 {
            return Err(invalid());
        }
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        for mut bytes in [header.as_bytes(), body.as_slice()] {
            while !bytes.is_empty() {
                self.stream.set_write_timeout(Some(remaining(deadline)?))?;
                let count = self.stream.write(bytes)?;
                if count == 0 {
                    return Err(io::ErrorKind::WriteZero.into());
                }
                bytes = &bytes[count..];
            }
        }
        self.stream.shutdown(Shutdown::Write)?;
        remaining(deadline)?;
        Ok(())
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        if let Ok(metadata) = fs::symlink_metadata(&self.socket)
            && metadata.file_type().is_socket()
            && (metadata.dev(), metadata.ino()) == self.identity
        {
            let _ = fs::remove_file(&self.socket);
        }
        if let Ok(bytes) = runtime::private_read(&self.dir.join("offer.json"), 4096)
            && let Ok(value) = serde_json::from_slice::<Value>(&bytes)
            && value["op"].as_str() == Some(&self.op)
        {
            let _ = publish(
                &self.dir,
                &json!({"schema_version":runtime::OFFER_SCHEMA,"state":"idle"}),
            );
        }
    }
}
