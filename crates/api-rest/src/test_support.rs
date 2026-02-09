use std::path::Path;
use std::sync::Mutex;

pub(crate) static ENV_LOCK: Mutex<()> = Mutex::new(());

pub(crate) struct EnvGuard {
    key: &'static str,
    original: Option<String>,
}

impl EnvGuard {
    pub(crate) fn set(key: &'static str, value: &str) -> Self {
        let original = std::env::var(key).ok();
        // SAFETY: test helper updates process env in scoped guard usage.
        unsafe { std::env::set_var(key, value) };
        Self { key, original }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.original {
            Some(value) => {
                // SAFETY: test helper restores process env in scoped guard usage.
                unsafe { std::env::set_var(self.key, value) };
            }
            None => {
                // SAFETY: test helper restores process env in scoped guard usage.
                unsafe { std::env::remove_var(self.key) };
            }
        }
    }
}

pub(crate) fn write_file(path: &Path, contents: &str) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(path, contents).expect("write");
}

pub(crate) fn write_json(path: &Path, value: &serde_json::Value) {
    write_file(path, &serde_json::to_string_pretty(value).unwrap());
}
