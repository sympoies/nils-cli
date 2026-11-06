//! Shared rendering vocabulary from session-notices-v1.md.

/// One kind's current label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoticeLabel {
    pub kind: &'static str,
    pub label: &'static str,
}

/// The sole kind-to-label table for summary rendering and history recognition.
pub const NOTICE_LABELS: &[NoticeLabel] = &[
    NoticeLabel {
        kind: "mailbox",
        label: "Mailbox",
    },
    NoticeLabel {
        kind: "mailbox.wake",
        label: "Mailbox",
    },
    NoticeLabel {
        kind: "mailbox.read",
        label: "Mailbox read",
    },
    NoticeLabel {
        kind: "mailbox.alert",
        label: "Mailbox alert",
    },
    NoticeLabel {
        kind: "mailbox.bounce",
        label: "Mailbox bounce",
    },
    NoticeLabel {
        kind: "schedule.patrol",
        label: "Patrol",
    },
    NoticeLabel {
        kind: "schedule.pulse",
        label: "Pulse",
    },
    NoticeLabel {
        kind: "schedule.tick",
        label: "Schedule",
    },
    NoticeLabel {
        kind: "compact.reminder",
        label: "Compact reminder",
    },
    NoticeLabel {
        kind: "compact.resume",
        label: "Compact resume",
    },
    NoticeLabel {
        kind: "runtime.resume",
        label: "Resume",
    },
    NoticeLabel {
        kind: "context.continue",
        label: "Continue",
    },
    NoticeLabel {
        kind: "dispatch.bootstrap",
        label: "Dispatch start",
    },
    NoticeLabel {
        kind: "dispatch.role",
        label: "Role dispatch",
    },
    NoticeLabel {
        kind: "session.handover",
        label: "Handover",
    },
    NoticeLabel {
        kind: "monitoring.service",
        label: "Monitoring",
    },
    NoticeLabel {
        kind: "monitoring.mirror",
        label: "Monitoring mirror",
    },
    NoticeLabel {
        kind: "monitoring.delivery",
        label: "Monitoring delivery",
    },
    NoticeLabel {
        kind: "role.restart",
        label: "Role restart",
    },
    NoticeLabel {
        kind: "role.unavailable",
        label: "Role unavailable",
    },
];

/// Unknown kinds keep their exact token; namespace prefixes have no meaning.
pub fn label_for(kind: &str) -> &str {
    NOTICE_LABELS
        .iter()
        .find(|entry| entry.kind == kind)
        .map_or(kind, |entry| entry.label)
}
