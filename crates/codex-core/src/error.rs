use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreErrorCategory {
    Config,
    Auth,
    Exec,
    Dependency,
    Validation,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderCategoryHint {
    Auth,
    Dependency,
    Validation,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct CoreError {
    pub category: CoreErrorCategory,
    pub code: &'static str,
    pub message: String,
    pub retryable: bool,
}

impl CoreError {
    pub fn new(
        category: CoreErrorCategory,
        code: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            category,
            code,
            message: message.into(),
            retryable: false,
        }
    }

    pub fn with_retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }

    pub fn config(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(CoreErrorCategory::Config, code, message)
    }

    pub fn auth(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(CoreErrorCategory::Auth, code, message)
    }

    pub fn exec(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(CoreErrorCategory::Exec, code, message)
    }

    pub fn dependency(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(CoreErrorCategory::Dependency, code, message)
    }

    pub fn validation(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(CoreErrorCategory::Validation, code, message)
    }

    pub fn internal(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(CoreErrorCategory::Internal, code, message)
    }

    pub fn exit_code_hint(&self) -> i32 {
        match self.category {
            CoreErrorCategory::Validation | CoreErrorCategory::Config => 64,
            CoreErrorCategory::Auth
            | CoreErrorCategory::Exec
            | CoreErrorCategory::Dependency
            | CoreErrorCategory::Internal => 1,
        }
    }

    pub fn provider_category_hint(&self) -> ProviderCategoryHint {
        match self.category {
            CoreErrorCategory::Auth => ProviderCategoryHint::Auth,
            CoreErrorCategory::Dependency => ProviderCategoryHint::Dependency,
            CoreErrorCategory::Validation | CoreErrorCategory::Config => {
                ProviderCategoryHint::Validation
            }
            CoreErrorCategory::Exec | CoreErrorCategory::Internal => ProviderCategoryHint::Internal,
        }
    }
}
