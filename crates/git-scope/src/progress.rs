use anyhow::Result;
use nils_term::progress::{Progress, ProgressFinish, ProgressOptions};

pub struct ProgressRunner {
    progress: Option<Progress>,
}

impl ProgressRunner {
    pub fn new(total: u64, enabled: bool) -> Self {
        let progress = if enabled && total > 0 {
            Some(Progress::new(
                total,
                ProgressOptions::default()
                    .with_prefix("git-scope ")
                    .with_finish(ProgressFinish::Clear),
            ))
        } else {
            None
        };

        Self { progress }
    }

    pub fn run<F>(&self, message: impl AsRef<str>, op: F) -> Result<()>
    where
        F: FnOnce() -> Result<()>,
    {
        match &self.progress {
            Some(progress) => {
                progress.set_message(message.as_ref());
                progress.suspend(op)?;
                progress.inc(1);
                Ok(())
            }
            None => op(),
        }
    }

    pub fn finish(&self) {
        if let Some(progress) = &self.progress {
            progress.finish_and_clear();
        }
    }
}
