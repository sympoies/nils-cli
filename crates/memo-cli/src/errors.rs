use serde::Serialize;

#[derive(Debug)]
pub struct AppError {
    exit_code: i32,
    code: Box<str>,
    message: Box<str>,
    details: Option<Box<serde_json::Value>>,
}

#[derive(Debug, Serialize)]
pub struct JsonError<'a> {
    pub code: &'a str,
    pub message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<&'a serde_json::Value>,
}

impl AppError {
    pub fn usage(message: impl Into<String>) -> Self {
        Self {
            exit_code: 64,
            code: "invalid-arguments".into(),
            message: message.into().into_boxed_str(),
            details: None,
        }
    }

    pub fn data(message: impl Into<String>) -> Self {
        Self {
            exit_code: 65,
            code: "invalid-input".into(),
            message: message.into().into_boxed_str(),
            details: None,
        }
    }

    pub fn runtime(message: impl Into<String>) -> Self {
        Self {
            exit_code: 1,
            code: "runtime-failure".into(),
            message: message.into().into_boxed_str(),
            details: None,
        }
    }

    pub fn db(err: rusqlite::Error) -> Self {
        Self::runtime(format!("database error: {err}"))
    }

    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = code.into().into_boxed_str();
        self
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(Box::new(details));
        self
    }

    pub fn exit_code(&self) -> i32 {
        self.exit_code
    }

    pub fn code(&self) -> &str {
        self.code.as_ref()
    }

    pub fn message(&self) -> &str {
        self.message.as_ref()
    }

    pub fn json_error(&self) -> JsonError<'_> {
        JsonError {
            code: self.code(),
            message: self.message(),
            details: self.details.as_deref(),
        }
    }
}

impl From<anyhow::Error> for AppError {
    fn from(value: anyhow::Error) -> Self {
        Self::runtime(value.to_string())
    }
}
