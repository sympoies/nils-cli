use super::*;
use crate::default_branch::{install::Installation, preflight::RepositoryState};

impl DefaultBranchCommitOptions {
    pub(crate) fn run(mut self, state: &RepositoryState, dry_run: bool) -> (i32, Option<String>) {
        self.options.summary_mode = SummaryMode::None;
        self.options.no_progress = true;
        self.options.quiet = true;
        self.options.dry_run = dry_run;
        self.options.require_clean = true;
        self.options.expect_head = Some(state.head.clone());
        self.options.repo = Some(state.root.clone());
        let mut installation = Installation::new(state);
        let code = run_options(self.options, Some(&mut installation));
        (code, installation.installed)
    }
}
