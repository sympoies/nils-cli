pub mod cli;
pub mod completion;
pub mod error;
pub mod run;
pub mod select;
pub mod test_mode;
pub mod types;

#[cfg(any(target_os = "macos", coverage))]
pub mod macos;

/// Linux backend (X11).
#[cfg(target_os = "linux")]
pub mod linux;
