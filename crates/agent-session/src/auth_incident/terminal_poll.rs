//! Cache only passive sampling evidence, never tmux input or runtime authority.
use jiff::Timestamp;

use crate::{SessionRecord, tmux_inventory::quiet_since_sample};

pub(crate) enum TerminalObservation {
    Live(PanePoll),
    Absent,
    Unavailable,
}

#[derive(Default)]
pub(crate) struct PanePoll {
    pub(crate) activity: Option<String>,
    pub(crate) sample: Option<PaneSample>,
}

pub(crate) struct PaneSample {
    launch_id: String,
    generation: u64,
    tmux_session: String,
    activity: String,
    activity_revision: Option<u64>,
    sampled_at: String,
}

impl PanePoll {
    pub(super) fn unchanged(
        &self,
        record: &SessionRecord,
        revision: Option<u64>,
        now: Timestamp,
    ) -> bool {
        let (Some(runtime), Some(sample)) = (&record.runtime, &self.sample) else {
            return false;
        };
        runtime.launch_id == sample.launch_id
            && runtime.generation == sample.generation
            && record.tmux_session == sample.tmux_session
            && revision == sample.activity_revision
            && self.activity.as_deref() == Some(sample.activity.as_str())
            && quiet_since_sample(self.activity.as_deref(), &sample.sampled_at, now)
    }

    pub(super) fn record_sample(
        &mut self,
        record: &SessionRecord,
        revision: Option<u64>,
        sampled_at: Timestamp,
    ) {
        self.sample =
            record
                .runtime
                .as_ref()
                .zip(self.activity.as_ref())
                .map(|(runtime, activity)| PaneSample {
                    launch_id: runtime.launch_id.clone(),
                    generation: runtime.generation,
                    tmux_session: record.tmux_session.clone(),
                    activity: activity.clone(),
                    activity_revision: revision,
                    sampled_at: sampled_at.to_string(),
                });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_incident::{sample, tests::record};
    use crate::{CliContext, session_dir, write_session_record};
    use pretty_assertions::assert_eq;
    use std::{fs, os::unix::fs::PermissionsExt};

    #[test]
    fn auth_fallback_samples_on_demand_and_drops_stale_incarnations() {
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        let mut record = record();
        fs::create_dir_all(session_dir(&context, &record.id)).unwrap();
        write_session_record(&context, &record).unwrap();
        let tmux = tmp.path().join("tmux");
        let calls = tmp.path().join("calls");
        fs::write(&tmux, format!(
            "#!/bin/sh\n[ \"$1\" = capture-pane ] || exit 1\nprintf 'capture-pane\\n' >> {}\nprintf 'idle\\n'\n",
            shell_words::quote(&calls.to_string_lossy()),
        )).unwrap();
        fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).unwrap();
        let activity = jiff::Timestamp::from_second(Timestamp::now().as_second() - 10).unwrap();
        let mut pane = PanePoll {
            activity: Some(activity.to_string()),
            ..Default::default()
        };

        sample(&context, &record, &tmux, &mut pane).unwrap();
        sample(&context, &record, &tmux, &mut pane).unwrap();
        assert_eq!(fs::read_to_string(&calls).unwrap().lines().count(), 1);
        assert!(
            !pane.unchanged(&record, Some(1), Timestamp::now()),
            "a hook revision must invalidate the sample"
        );
        let expired = jiff::Timestamp::from_second(
            Timestamp::now().as_second() + crate::tmux_inventory::QUIET_REPROBE_SECONDS,
        )
        .unwrap();
        assert!(
            !pane.unchanged(&record, None, expired),
            "an idle sample must expire"
        );

        pane.activity = Some(
            jiff::Timestamp::from_second(activity.as_second() + 1)
                .unwrap()
                .to_string(),
        );
        sample(&context, &record, &tmux, &mut pane).unwrap();
        assert_eq!(fs::read_to_string(&calls).unwrap().lines().count(), 2);

        record.runtime.as_mut().unwrap().launch_id = "replacement".into();
        write_session_record(&context, &record).unwrap();
        sample(&context, &record, &tmux, &mut pane).unwrap();
        assert_eq!(fs::read_to_string(&calls).unwrap().lines().count(), 3);
    }
}
