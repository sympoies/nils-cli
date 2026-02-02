use nils_term::progress::Progress;

pub(super) struct SuiteProgress {
    progress: Option<Progress>,
    touched: bool,
}

impl SuiteProgress {
    pub(super) fn new(progress: Option<Progress>) -> Self {
        Self {
            progress,
            touched: false,
        }
    }

    pub(super) fn on_case_start(&mut self, position: u64, message: &str) {
        let Some(progress) = self.progress.as_ref() else {
            return;
        };

        self.touched = true;
        progress.set_position(position);
        progress.set_message(message);
    }

    fn finish(&mut self) {
        if !self.touched {
            return;
        }

        if let Some(progress) = self.progress.as_ref() {
            progress.finish();
        }
    }
}

impl Drop for SuiteProgress {
    fn drop(&mut self) {
        self.finish();
    }
}
