use gemini_cli::{rate_limits, starship};

use std::ffi::{OsStr, OsString};
use std::fs as stdfs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    match LOCK.get_or_init(|| Mutex::new(())).lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

struct EnvGuard {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvGuard {
    fn set(key: &'static str, value: impl AsRef<OsStr>) -> Self {
        let previous = std::env::var_os(key);
        // SAFETY: tests serialize env mutations via env_lock.
        unsafe { std::env::set_var(key, value) };
        Self { key, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        if let Some(value) = self.previous.take() {
            // SAFETY: tests serialize env mutations via env_lock.
            unsafe { std::env::set_var(self.key, value) };
        } else {
            // SAFETY: tests serialize env mutations via env_lock.
            unsafe { std::env::remove_var(self.key) };
        }
    }
}

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!(
            "nils-gemini-cli-{label}-{}-{nanos}",
            std::process::id()
        ));
        let _ = stdfs::remove_dir_all(&path);
        stdfs::create_dir_all(&path).expect("temp dir");
        Self { path }
    }

    fn join(&self, child: &str) -> PathBuf {
        self.path.join(child)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = stdfs::remove_dir_all(&self.path);
    }
}

fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or(0)
}

fn write_auth_secret(dir: &TestDir) -> (PathBuf, PathBuf, PathBuf) {
    let secrets = dir.join("secrets");
    stdfs::create_dir_all(&secrets).expect("secrets");
    let secrets = stdfs::canonicalize(&secrets).expect("canonical secrets");
    let auth_file = secrets.join("alpha.json");
    stdfs::write(
        &auth_file,
        r#"{"tokens":{"access_token":"tok","account_id":"acct_001"}}"#,
    )
    .expect("write auth");

    let cache_root = dir.join("cache-root");
    stdfs::create_dir_all(&cache_root).expect("cache root");
    let cache_root = stdfs::canonicalize(&cache_root).expect("canonical cache root");
    (auth_file, secrets, cache_root)
}

fn usage_body() -> &'static str {
    r#"{
  "rate_limit": {
    "primary_window": { "limit_window_seconds": 18000, "used_percent": 6, "reset_at": 1700003600 },
    "secondary_window": { "limit_window_seconds": 604800, "used_percent": 12, "reset_at": 1700600000 }
  }
}"#
}

fn handle_connection(stream: &mut TcpStream, response_body: &str) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
    let mut buf = [0u8; 2048];
    let _ = stream.read(&mut buf);

    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response_body.len(),
        response_body
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn spawn_usage_server() -> (String, thread::JoinHandle<usize>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    listener
        .set_nonblocking(true)
        .expect("set listener nonblocking");
    let addr = listener.local_addr().expect("local addr");
    let body = usage_body().to_string();

    let handle = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    handle_connection(&mut stream, &body);
                    return 1;
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return 0,
            }
        }
        0
    });

    (format!("http://{addr}/"), handle)
}

#[test]
fn starship_refresh_updates_cache() {
    let _lock = env_lock();
    let dir = TestDir::new("starship-refresh-updates-cache");
    let (auth_file, secrets, cache_root) = write_auth_secret(&dir);

    let (base_url, server) = spawn_usage_server();

    let _auth = EnvGuard::set("GEMINI_AUTH_FILE", &auth_file);
    let _secret_dir = EnvGuard::set("GEMINI_SECRET_DIR", &secrets);
    let _cache_root = EnvGuard::set("ZSH_CACHE_DIR", &cache_root);
    let _enabled = EnvGuard::set("GEMINI_STARSHIP_ENABLED", "true");
    let _base = EnvGuard::set("GEMINI_CHATGPT_BASE_URL", &base_url);
    let _connect = EnvGuard::set("GEMINI_STARSHIP_CURL_CONNECT_TIMEOUT_SECONDS", "1");
    let _max_time = EnvGuard::set("GEMINI_STARSHIP_CURL_MAX_TIME_SECONDS", "3");

    let options = starship::StarshipOptions {
        refresh: true,
        time_format: Some("%Y-%m-%dT%H:%MZ".to_string()),
        ..Default::default()
    };
    assert_eq!(starship::run(&options), 0);
    assert_eq!(server.join().expect("server join"), 1);

    let cache_file = rate_limits::cache_file_for_target(&auth_file).expect("cache file");
    let content = stdfs::read_to_string(cache_file).expect("cache content");
    assert!(content.contains("non_weekly_remaining=94"));
    assert!(content.contains("weekly_remaining=88"));
}

#[test]
fn starship_stale_cached_entry_refreshes_on_run() {
    let _lock = env_lock();
    let dir = TestDir::new("starship-stale-cache-refreshes");
    let (auth_file, secrets, cache_root) = write_auth_secret(&dir);

    let (base_url, server) = spawn_usage_server();

    let _auth = EnvGuard::set("GEMINI_AUTH_FILE", &auth_file);
    let _secret_dir = EnvGuard::set("GEMINI_SECRET_DIR", &secrets);
    let _cache_root = EnvGuard::set("ZSH_CACHE_DIR", &cache_root);
    let _enabled = EnvGuard::set("GEMINI_STARSHIP_ENABLED", "true");
    let _base = EnvGuard::set("GEMINI_CHATGPT_BASE_URL", &base_url);
    let _connect = EnvGuard::set("GEMINI_STARSHIP_CURL_CONNECT_TIMEOUT_SECONDS", "1");
    let _max_time = EnvGuard::set("GEMINI_STARSHIP_CURL_MAX_TIME_SECONDS", "3");

    let stale_fetched = now_epoch().saturating_sub(10).max(1);
    rate_limits::write_starship_cache(
        &auth_file,
        stale_fetched,
        "5h",
        1,
        2,
        1700600000,
        Some(1700003600),
    )
    .expect("write stale cache");

    let options = starship::StarshipOptions {
        ttl: Some("1s".to_string()),
        time_format: Some("%Y-%m-%dT%H:%MZ".to_string()),
        ..Default::default()
    };
    assert_eq!(starship::run(&options), 0);
    assert_eq!(server.join().expect("server join"), 1);

    let cache_file = rate_limits::cache_file_for_target(&auth_file).expect("cache file");
    let content = stdfs::read_to_string(cache_file).expect("cache content");
    assert!(content.contains("non_weekly_remaining=94"));
    assert!(content.contains("weekly_remaining=88"));
}
