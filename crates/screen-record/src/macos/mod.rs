#[cfg(not(coverage))]
pub mod permissions;
#[cfg(not(coverage))]
pub mod screenshot;
#[cfg(not(coverage))]
pub mod shareable;
#[cfg(not(coverage))]
pub mod stream;
#[cfg(not(coverage))]
pub mod writer;

#[cfg(coverage)]
pub mod permissions {
    use crate::error::CliError;

    pub fn preflight() -> Result<(), CliError> {
        Ok(())
    }

    pub fn request_permission() -> Result<(), CliError> {
        Ok(())
    }
}

#[cfg(coverage)]
pub mod shareable {
    use crate::error::CliError;
    use crate::test_mode;
    use crate::types::ShareableContent;

    pub fn fetch_shareable() -> Result<ShareableContent, CliError> {
        Ok(test_mode::shareable_content())
    }
}

#[cfg(coverage)]
pub mod screenshot {
    use std::path::Path;

    use crate::cli::ImageFormat;
    use crate::error::CliError;
    use crate::test_mode;
    use crate::types::WindowInfo;

    pub fn screenshot_window(
        _window: &WindowInfo,
        path: &Path,
        format: ImageFormat,
    ) -> Result<(), CliError> {
        test_mode::screenshot_fixture(path, format)
    }
}

#[cfg(coverage)]
pub mod stream {
    use std::path::Path;

    use crate::cli::{AudioMode, ContainerFormat};
    use crate::error::CliError;
    use crate::test_mode;
    use crate::types::WindowInfo;

    pub fn record_window(
        _window: &WindowInfo,
        _duration: u64,
        _audio: AudioMode,
        path: &Path,
        format: ContainerFormat,
    ) -> Result<(), CliError> {
        test_mode::record_fixture(path, format)
    }

    pub fn record_display(
        _display_id: u32,
        _duration: u64,
        _audio: AudioMode,
        path: &Path,
        format: ContainerFormat,
    ) -> Result<(), CliError> {
        test_mode::record_fixture(path, format)
    }

    pub fn record_main_display(
        duration: u64,
        audio: AudioMode,
        path: &Path,
        format: ContainerFormat,
    ) -> Result<(), CliError> {
        record_display(0, duration, audio, path, format)
    }
}
