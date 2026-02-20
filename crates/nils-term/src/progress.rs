use std::io::{self, IsTerminal};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use indicatif::{ProgressBar, ProgressStyle};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressEnabled {
    /// Enable progress only when the selected draw target is a TTY.
    ///
    /// Notes:
    /// - With the default draw target (stderr), this disables progress when stderr is not a TTY,
    ///   keeping stdout clean for piping and machine-readable output.
    /// - With `ProgressDrawTarget::to_writer(...)` (tests), auto-enable is never blocked by TTY
    ///   detection.
    Auto,
    On,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressFinish {
    /// Leave the progress line visible when finished.
    Leave,
    /// Clear the progress line when finished.
    Clear,
}

#[derive(Debug, Clone)]
pub struct ProgressOptions {
    pub enabled: ProgressEnabled,
    pub prefix: String,
    /// Fixed terminal width used by writer-based draw targets (tests).
    pub width: Option<u16>,
    pub finish: ProgressFinish,
    pub draw_target: ProgressDrawTarget,
}

impl Default for ProgressOptions {
    fn default() -> Self {
        Self {
            enabled: ProgressEnabled::Auto,
            prefix: String::new(),
            width: None,
            finish: ProgressFinish::Leave,
            draw_target: ProgressDrawTarget::stderr(),
        }
    }
}

impl ProgressOptions {
    pub fn with_enabled(mut self, enabled: ProgressEnabled) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self
    }

    pub fn with_width(mut self, width: Option<u16>) -> Self {
        self.width = width;
        self
    }

    pub fn with_finish(mut self, finish: ProgressFinish) -> Self {
        self.finish = finish;
        self
    }

    pub fn with_draw_target(mut self, draw_target: ProgressDrawTarget) -> Self {
        self.draw_target = draw_target;
        self
    }
}

#[derive(Debug, Clone)]
pub enum ProgressDrawTarget {
    /// Draw to stderr (default).
    Stderr,
    /// Draw to an in-memory writer (intended for deterministic tests).
    Writer { buffer: Arc<Mutex<Vec<u8>>> },
}

impl ProgressDrawTarget {
    pub fn stderr() -> Self {
        Self::Stderr
    }

    pub fn to_writer(buffer: Arc<Mutex<Vec<u8>>>) -> Self {
        Self::Writer { buffer }
    }
}

#[derive(Debug, Clone)]
pub struct Progress {
    state: Option<Arc<ProgressState>>,
}

#[derive(Debug)]
struct ProgressState {
    bar: ProgressBar,
    finish: ProgressFinish,
    rendered: AtomicBool,
    finished: AtomicBool,
}

impl Drop for ProgressState {
    fn drop(&mut self) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if self.finished.load(Ordering::Relaxed) {
                return;
            }
            if !self.rendered.load(Ordering::Relaxed) {
                return;
            }
            match self.finish {
                ProgressFinish::Leave => self.bar.finish(),
                ProgressFinish::Clear => self.bar.finish_and_clear(),
            }
        }));
    }
}

impl Progress {
    /// Create a determinate progress bar.
    pub fn new(total: u64, options: ProgressOptions) -> Self {
        if !should_enable(&options) {
            return Self { state: None };
        }

        let draw_target = to_indicatif_draw_target(&options.draw_target, options.width);

        let bar = ProgressBar::new(total);
        bar.set_draw_target(draw_target);
        bar.set_style(determinate_style());

        let state = Arc::new(ProgressState {
            bar,
            finish: options.finish,
            rendered: AtomicBool::new(false),
            finished: AtomicBool::new(false),
        });

        if !options.prefix.is_empty() {
            state.rendered.store(true, Ordering::Relaxed);
            state.bar.set_prefix(options.prefix);
        }

        Self { state: Some(state) }
    }

    /// Create a spinner progress indicator.
    pub fn spinner(options: ProgressOptions) -> Self {
        if !should_enable(&options) {
            return Self { state: None };
        }

        let draw_target = to_indicatif_draw_target(&options.draw_target, options.width);

        let bar = ProgressBar::new_spinner();
        bar.set_draw_target(draw_target);
        bar.set_style(spinner_style());

        let state = Arc::new(ProgressState {
            bar,
            finish: options.finish,
            rendered: AtomicBool::new(false),
            finished: AtomicBool::new(false),
        });

        if !options.prefix.is_empty() {
            state.rendered.store(true, Ordering::Relaxed);
            state.bar.set_prefix(options.prefix);
        }

        Self { state: Some(state) }
    }

    pub fn set_position(&self, pos: u64) {
        if let Some(state) = &self.state {
            state.rendered.store(true, Ordering::Relaxed);
            state.bar.set_position(pos);
        }
    }

    pub fn inc(&self, delta: u64) {
        if let Some(state) = &self.state {
            state.rendered.store(true, Ordering::Relaxed);
            state.bar.inc(delta);
        }
    }

    pub fn tick(&self) {
        if let Some(state) = &self.state {
            state.rendered.store(true, Ordering::Relaxed);
            state.bar.tick();
        }
    }

    pub fn set_message(&self, message: impl Into<String>) {
        if let Some(state) = &self.state {
            state.rendered.store(true, Ordering::Relaxed);
            state.bar.set_message(message.into());
        }
    }

    /// Finish and leave the progress output visible.
    pub fn finish(&self) {
        if let Some(state) = &self.state {
            if state.finished.swap(true, Ordering::Relaxed) {
                return;
            }
            state.rendered.store(true, Ordering::Relaxed);
            state.bar.finish();
        }
    }

    pub fn finish_with_message(&self, message: impl Into<String>) {
        if let Some(state) = &self.state {
            if state.finished.swap(true, Ordering::Relaxed) {
                return;
            }
            state.rendered.store(true, Ordering::Relaxed);
            state.bar.finish_with_message(message.into());
        }
    }

    /// Finish and clear the progress output.
    pub fn finish_and_clear(&self) {
        if let Some(state) = &self.state {
            if state.finished.swap(true, Ordering::Relaxed) {
                return;
            }
            state.rendered.store(true, Ordering::Relaxed);
            state.bar.finish_and_clear();
        }
    }

    pub fn suspend<F: FnOnce() -> R, R>(&self, f: F) -> R {
        match &self.state {
            Some(state) => state.bar.suspend(f),
            None => f(),
        }
    }
}

fn should_enable(options: &ProgressOptions) -> bool {
    match options.enabled {
        ProgressEnabled::On => true,
        ProgressEnabled::Off => false,
        ProgressEnabled::Auto => match &options.draw_target {
            ProgressDrawTarget::Stderr => io::stderr().is_terminal(),
            ProgressDrawTarget::Writer { .. } => true,
        },
    }
}

fn determinate_style() -> ProgressStyle {
    // Keep this stable and non-panicking even if indicatif changes behavior.
    let style = ProgressStyle::with_template("{prefix}{wide_bar} {pos}/{len} {msg}");
    match style {
        Ok(style) => style.progress_chars("#-"),
        Err(_) => ProgressStyle::default_bar().progress_chars("#-"),
    }
}

fn spinner_style() -> ProgressStyle {
    let style = ProgressStyle::with_template("{prefix}{spinner} {msg}");
    match style {
        Ok(style) => style.tick_chars(r"-\|/"),
        Err(_) => ProgressStyle::default_spinner().tick_chars(r"-\|/"),
    }
}

fn to_indicatif_draw_target(
    draw_target: &ProgressDrawTarget,
    width: Option<u16>,
) -> indicatif::ProgressDrawTarget {
    match draw_target {
        ProgressDrawTarget::Stderr => indicatif::ProgressDrawTarget::stderr(),
        ProgressDrawTarget::Writer { buffer } => indicatif::ProgressDrawTarget::term_like(
            Box::new(WriterTerm::new(buffer.clone(), width.unwrap_or(80))),
        ),
    }
}

#[derive(Debug)]
struct WriterTerm {
    buffer: Arc<Mutex<Vec<u8>>>,
    width: u16,
}

impl WriterTerm {
    fn new(buffer: Arc<Mutex<Vec<u8>>>, width: u16) -> Self {
        Self { buffer, width }
    }

    fn write_all(&self, bytes: &[u8]) -> io::Result<()> {
        let mut guard = match self.buffer.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.extend_from_slice(bytes);
        Ok(())
    }

    fn write_str_bytes(&self, s: &str) -> io::Result<()> {
        self.write_all(s.as_bytes())
    }
}

impl indicatif::TermLike for WriterTerm {
    fn width(&self) -> u16 {
        self.width
    }

    fn move_cursor_up(&self, n: usize) -> io::Result<()> {
        self.write_str_bytes(&format!("\u{1b}[{n}A"))
    }

    fn move_cursor_down(&self, n: usize) -> io::Result<()> {
        self.write_str_bytes(&format!("\u{1b}[{n}B"))
    }

    fn move_cursor_right(&self, n: usize) -> io::Result<()> {
        self.write_str_bytes(&format!("\u{1b}[{n}C"))
    }

    fn move_cursor_left(&self, n: usize) -> io::Result<()> {
        self.write_str_bytes(&format!("\u{1b}[{n}D"))
    }

    fn write_line(&self, s: &str) -> io::Result<()> {
        self.write_str_bytes(s)?;
        self.write_all(b"\n")
    }

    fn write_str(&self, s: &str) -> io::Result<()> {
        self.write_str_bytes(s)
    }

    fn clear_line(&self) -> io::Result<()> {
        self.write_str_bytes("\r\u{1b}[2K")
    }

    fn flush(&self) -> io::Result<()> {
        Ok(())
    }
}
