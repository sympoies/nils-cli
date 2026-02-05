use std::fmt;

#[derive(Debug, Clone)]
pub struct CliError {
    message: String,
    exit_code: u8,
}

impl CliError {
    pub fn usage(message: impl Into<String>) -> Self {
        Self::new(message, 2)
    }

    pub fn unsupported_platform() -> Self {
        Self::usage("screen-record is only supported on macOS (12+) and Linux (X11)")
    }

    pub fn runtime(message: impl Into<String>) -> Self {
        Self::new(message, 1)
    }

    fn new(message: impl Into<String>, exit_code: u8) -> Self {
        let mut message = message.into();
        if !message.starts_with("error:") {
            message = format!("error: {message}");
        }
        Self { message, exit_code }
    }

    pub fn exit_code(&self) -> u8 {
        self.exit_code
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for CliError {}
