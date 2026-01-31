use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphqlOperationFile {
    pub path: PathBuf,
    pub operation: String,
}

impl GraphqlOperationFile {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let operation = std::fs::read_to_string(path)
            .with_context(|| format!("read operation file: {}", path.display()))?;
        Ok(Self {
            path: std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()),
            operation,
        })
    }
}
