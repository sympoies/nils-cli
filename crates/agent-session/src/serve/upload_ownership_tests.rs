//! Watch fixture I/O so refusal tests also detect premature content reads.
use super::*;
use pretty_assertions::assert_eq;
use std::io::{Read, Write};
use std::os::{fd::FromRawFd, unix::ffi::OsStrExt};

struct AccessWatch(fs::File);

impl AccessWatch {
    fn new(path: &Path, mask: u32) -> Self {
        let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        assert!(fd >= 0);
        let file = unsafe { fs::File::from_raw_fd(fd) };
        let path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        assert!(unsafe { libc::inotify_add_watch(fd, path.as_ptr(), mask) } >= 0);
        Self(file)
    }

    fn events(&mut self) -> usize {
        let mut bytes = [0; 4096];
        match self.0.read(&mut bytes) {
            Ok(n) => n / std::mem::size_of::<libc::inotify_event>(),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => 0,
            Err(e) => panic!("inotify read: {e}"),
        }
    }
}

async fn assert_refused(fixture: &Fixture, path: &Path) {
    let mut watch = AccessWatch::new(path, libc::IN_ACCESS);
    fixture.append(json!([{"type":"input_text","text":format!("'{}'", path.display())}]));
    let page = fixture.refresh().await;
    let messages = page["data"]["messages"].as_array().unwrap();
    let message = messages.last().unwrap();
    assert!(message["images"].as_array().is_none_or(Vec::is_empty));
    assert!(
        message["parts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["kind"] != "file" && p["kind"] != "image")
    );
    assert_eq!(watch.events(), 0, "refusal must precede content reads");
}

#[tokio::test]
async fn upload_ownership_refuses_over_size_before_reading() {
    for extension in ["txt", "png"] {
        let fixture = Fixture::new();
        let path = fixture
            .upload("own", &format!("large.{extension}"), &fixture.png)
            .await;
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(8 * 1024 * 1024 + 1)
            .unwrap();
        assert_refused(&fixture, &path).await;
    }
}

#[tokio::test]
async fn upload_ownership_refuses_directory_before_reading() {
    for extension in ["txt", "png"] {
        let fixture = Fixture::new();
        let path = fixture
            .upload("own", &format!("directory.{extension}"), &fixture.png)
            .await;
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert_refused(&fixture, &path).await;
    }
}

#[tokio::test]
async fn upload_ownership_refuses_fifo_before_reading() {
    for extension in ["txt", "png"] {
        let fixture = Fixture::new();
        let path = fixture
            .upload("own", &format!("pipe.{extension}"), &fixture.png)
            .await;
        fs::remove_file(&path).unwrap();
        let cpath = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) }, 0);
        let mut pipe = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&path)
            .unwrap();
        pipe.write_all(&fixture.png).unwrap();
        assert_refused(&fixture, &path).await;
        let mut remaining = vec![0; fixture.png.len()];
        pipe.read_exact(&mut remaining).unwrap();
        assert_eq!(remaining, fixture.png);
    }
}

#[tokio::test]
async fn upload_ownership_accepts_exact_size_cap() {
    let fixture = Fixture::new();
    let path = fixture.upload("own", "limit.txt", b"content").await;
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(8 * 1024 * 1024)
        .unwrap();
    let mut watch = AccessWatch::new(&path, libc::IN_ACCESS);
    fixture.append(json!([{"type":"input_text","text":format!("'{}'", path.display())}]));
    let page = fixture.refresh().await;
    let parts = page["data"]["messages"][0]["parts"].as_array().unwrap();
    let files: Vec<_> = parts.iter().filter(|p| p["kind"] == "file").collect();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["bytes"], 8 * 1024 * 1024);
    assert_eq!(files[0]["original_name"], "limit.txt");
    assert_eq!(watch.events(), 0);
}

#[tokio::test]
async fn upload_ownership_resolves_once_per_history_read() {
    let fixture = Fixture::new();
    let owned = fixture.upload("own", "owned.txt", b"owned").await;
    let foreign = fixture.upload("other", "foreign.txt", b"foreign").await;
    let paths = (0..3000)
        .map(|i| {
            format!(
                "'{}'",
                foreign.with_file_name(format!("missing-{i}.txt")).display()
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    fixture.append(json!([{"type":"input_text","text":format!("{paths} '{}'", owned.display())}]));
    // Warm catalog discovery separately; measure a real history read through HTTP.
    let _ = fixture.refresh().await;
    let record = foreign
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("session.json");
    let mut watch = AccessWatch::new(&record, libc::IN_OPEN | libc::IN_CLOSE_NOWRITE);
    let start = std::time::Instant::now();
    let page = fixture.refresh().await;
    eprintln!(
        "3000 managed-shape paths history read: {:?}",
        start.elapsed()
    );
    let parts = page["data"]["messages"][0]["parts"].as_array().unwrap();
    assert_eq!(parts.iter().filter(|p| p["kind"] == "file").count(), 1);
    // Open and close for one ownership proof; no per-path session-record reads.
    assert!(watch.events() <= 2, "ownership was resolved repeatedly");
}

#[tokio::test]
async fn upload_ownership_is_rechecked_on_the_next_history_read() {
    let fixture = Fixture::new();
    let path = fixture.upload("own", "notes.txt", b"owned").await;
    fixture.append(json!([{"type":"input_text","text":format!("'{}'", path.display())}]));
    let first = fixture.refresh().await;
    assert_eq!(
        first["data"]["messages"][0]["parts"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["kind"] == "file")
            .count(),
        1
    );
    let mut record = crate::load_session_record(&fixture.state.context, "own").unwrap();
    record.provider_resume.as_mut().unwrap().session_id = "different-provider-session".into();
    crate::write_session_record(&fixture.state.context, &record).unwrap();
    let second = fixture.refresh().await;
    assert!(
        second["data"]["messages"][0]["parts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["kind"] != "file")
    );
}
