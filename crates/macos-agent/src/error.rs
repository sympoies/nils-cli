use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCategory {
    Usage,
    Runtime,
}

impl ErrorCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Usage => "usage",
            Self::Runtime => "runtime",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CliError {
    message: String,
    exit_code: u8,
    category: ErrorCategory,
    operation: Option<String>,
    hints: Vec<String>,
}

impl CliError {
    pub fn usage(message: impl Into<String>) -> Self {
        Self::new(message, 2, ErrorCategory::Usage)
    }

    pub fn runtime(message: impl Into<String>) -> Self {
        Self::new(message, 1, ErrorCategory::Runtime)
    }

    pub fn unsupported_platform() -> Self {
        Self::usage("macos-agent is only supported on macOS")
    }

    pub fn timeout(operation: &str, timeout_ms: u64) -> Self {
        Self::runtime(format!("{operation} timed out after {timeout_ms}ms"))
            .with_operation(operation)
            .with_hint(
                "Increase --timeout-ms for slower apps or enable --retries for transient failures.",
            )
    }

    pub fn exit_code(&self) -> u8 {
        self.exit_code
    }

    pub fn category(&self) -> ErrorCategory {
        self.category
    }

    pub fn operation(&self) -> Option<&str> {
        self.operation.as_deref()
    }

    pub fn hints(&self) -> &[String] {
        &self.hints
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        let hint = hint.into();
        if !hint.trim().is_empty() {
            self.hints.push(hint);
        }
        self
    }

    pub fn with_operation(mut self, operation: impl Into<String>) -> Self {
        let operation = operation.into();
        if !operation.trim().is_empty() {
            self.operation = Some(operation);
        }
        self
    }
}

impl CliError {
    fn new(message: impl Into<String>, exit_code: u8, category: ErrorCategory) -> Self {
        let message = message
            .into()
            .trim()
            .trim_start_matches("error:")
            .trim()
            .to_string();
        Self {
            message,
            exit_code,
            category,
            operation: None,
            hints: Vec::new(),
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "error: {}", self.message)
    }
}

impl std::error::Error for CliError {}
