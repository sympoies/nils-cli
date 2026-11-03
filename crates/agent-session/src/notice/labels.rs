//! Shared rendering and stored-title vocabulary from session-notices-v1.md.

/// One kind's current label and, when different, its stored transcript title.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoticeLabel {
    pub kind: &'static str,
    pub label: &'static str,
    pub stored_title: Option<&'static str>,
}

/// The sole kind-to-label table for summary rendering and history recognition.
pub const NOTICE_LABELS: &[NoticeLabel] = &[
    NoticeLabel {
        kind: "mailbox",
        label: "Mailbox",
        stored_title: None,
    },
    NoticeLabel {
        kind: "mailbox.wake",
        label: "Mailbox",
        stored_title: Some("Mailbox wake"),
    },
    NoticeLabel {
        kind: "mailbox.read",
        label: "Mailbox read",
        stored_title: None,
    },
    NoticeLabel {
        kind: "mailbox.alert",
        label: "Mailbox alert",
        stored_title: None,
    },
    NoticeLabel {
        kind: "mailbox.bounce",
        label: "Mailbox bounce",
        stored_title: None,
    },
    NoticeLabel {
        kind: "schedule.patrol",
        label: "Patrol",
        stored_title: None,
    },
    NoticeLabel {
        kind: "schedule.pulse",
        label: "Pulse",
        stored_title: None,
    },
    NoticeLabel {
        kind: "schedule.tick",
        label: "Schedule",
        stored_title: None,
    },
    NoticeLabel {
        kind: "compact.reminder",
        label: "Compact reminder",
        stored_title: None,
    },
    NoticeLabel {
        kind: "compact.resume",
        label: "Compact resume",
        stored_title: None,
    },
    NoticeLabel {
        kind: "runtime.resume",
        label: "Resume",
        stored_title: None,
    },
    NoticeLabel {
        kind: "context.continue",
        label: "Continue",
        stored_title: None,
    },
    NoticeLabel {
        kind: "dispatch.bootstrap",
        label: "Dispatch start",
        stored_title: None,
    },
    NoticeLabel {
        kind: "dispatch.role",
        label: "Role dispatch",
        stored_title: None,
    },
    NoticeLabel {
        kind: "session.handover",
        label: "Handover",
        stored_title: None,
    },
    NoticeLabel {
        kind: "monitoring.service",
        label: "Monitoring",
        stored_title: None,
    },
    NoticeLabel {
        kind: "monitoring.mirror",
        label: "Monitoring mirror",
        stored_title: None,
    },
    NoticeLabel {
        kind: "monitoring.delivery",
        label: "Monitoring delivery",
        stored_title: None,
    },
    NoticeLabel {
        kind: "role.restart",
        label: "Role restart",
        stored_title: None,
    },
    NoticeLabel {
        kind: "role.unavailable",
        label: "Role unavailable",
        stored_title: None,
    },
];

/// Unknown kinds keep their exact token; namespace prefixes have no meaning.
pub fn label_for(kind: &str) -> &str {
    NOTICE_LABELS
        .iter()
        .find(|entry| entry.kind == kind)
        .map_or(kind, |entry| entry.label)
}
