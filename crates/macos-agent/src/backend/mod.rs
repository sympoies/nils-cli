pub mod applescript;
pub mod cliclick;
pub mod input_source;
pub mod process;

use crate::backend::process::ProcessRunner;
use crate::error::CliError;
use crate::model::{
    AxClickRequest, AxClickResult, AxListRequest, AxListResult, AxTypeRequest, AxTypeResult,
};

pub trait AxBackendAdapter {
    fn list(
        &self,
        runner: &dyn ProcessRunner,
        request: &AxListRequest,
        timeout_ms: u64,
    ) -> Result<AxListResult, CliError>;

    fn click(
        &self,
        runner: &dyn ProcessRunner,
        request: &AxClickRequest,
        timeout_ms: u64,
    ) -> Result<AxClickResult, CliError>;

    fn type_text(
        &self,
        runner: &dyn ProcessRunner,
        request: &AxTypeRequest,
        timeout_ms: u64,
    ) -> Result<AxTypeResult, CliError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct AppleScriptAxBackend;

impl AxBackendAdapter for AppleScriptAxBackend {
    fn list(
        &self,
        runner: &dyn ProcessRunner,
        request: &AxListRequest,
        timeout_ms: u64,
    ) -> Result<AxListResult, CliError> {
        applescript::ax_list(runner, request, timeout_ms)
    }

    fn click(
        &self,
        runner: &dyn ProcessRunner,
        request: &AxClickRequest,
        timeout_ms: u64,
    ) -> Result<AxClickResult, CliError> {
        applescript::ax_click(runner, request, timeout_ms)
    }

    fn type_text(
        &self,
        runner: &dyn ProcessRunner,
        request: &AxTypeRequest,
        timeout_ms: u64,
    ) -> Result<AxTypeResult, CliError> {
        applescript::ax_type(runner, request, timeout_ms)
    }
}
