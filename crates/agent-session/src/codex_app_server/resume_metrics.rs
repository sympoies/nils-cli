//! One initial restore observation on the managed TUI connection.
use super::*;
use crate::startup::resume_metrics::{self as timing, FinishOutcome, Sample};

enum Restore {
    Inactive,
    AwaitingRequest,
    AwaitingResponse(String),
}

pub(super) struct Observation {
    restore: Restore,
    // The receipt enforces its own gate-relative deadline. This only bounds
    // how long a stalled proxy keeps sampling ingress.
    observe_until: Instant,
    writer: Option<tokio::task::JoinHandle<()>>,
}

impl Observation {
    pub(super) fn for_runtime(context: &CliContext, record: &SessionRecord) -> Self {
        Self {
            restore: if timing::pending(context, record) {
                Restore::AwaitingRequest
            } else {
                Restore::Inactive
            },
            observe_until: Instant::now() + timing::WINDOW,
            writer: None,
        }
    }

    pub(super) fn client(&mut self, record: &SessionRecord, value: &Value) {
        match &self.restore {
            Restore::AwaitingRequest => match value.get("method").and_then(Value::as_str) {
                Some("thread/resume") => {
                    self.restore = record
                        .provider_resume
                        .as_ref()
                        .filter(|resume| {
                            value.pointer("/params/threadId").and_then(Value::as_str)
                                == Some(resume.session_id.as_str())
                        })
                        .and_then(|_| value.get("id").and_then(json_id_key))
                        .map_or(Restore::Inactive, Restore::AwaitingResponse);
                }
                Some("thread/start") => self.restore = Restore::Inactive,
                _ => {}
            },
            Restore::AwaitingResponse(id)
                if value.get("method").is_some()
                    && value.get("id").and_then(json_id_key).as_ref() == Some(id) =>
            {
                self.restore = Restore::Inactive;
            }
            _ => {}
        }
    }

    pub(super) fn ingress(&self) -> Option<Sample> {
        (matches!(self.restore, Restore::AwaitingResponse(_))
            && Instant::now() < self.observe_until)
            .then(Sample::now)
    }

    pub(super) fn server(
        &mut self,
        context: &CliContext,
        record: &SessionRecord,
        value: &Value,
        observed: Option<Sample>,
    ) {
        let (Restore::AwaitingResponse(request), Some(observed)) = (&self.restore, observed) else {
            return;
        };
        if json_rpc_response_id_key(value).as_ref() != Some(request) {
            return;
        }
        self.restore = Restore::Inactive;
        let outcome = if value.get("error").is_some() {
            FinishOutcome::ProviderFailed
        } else if record.provider_resume.as_ref().is_some_and(|resume| {
            value.pointer("/result/thread/id").and_then(Value::as_str)
                == Some(resume.session_id.as_str())
        }) {
            FinishOutcome::Succeeded
        } else {
            return;
        };
        let context = context.clone();
        let record = record.clone();
        self.writer = Some(tokio::task::spawn_blocking(move || {
            timing::finish(&context, &record, outcome, observed);
        }));
    }

    pub(super) async fn finish(self) {
        if let Some(writer) = self.writer {
            let _ = writer.await;
        }
    }
}
