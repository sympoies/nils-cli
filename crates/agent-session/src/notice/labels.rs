//! Shared rendering vocabulary from session-notices-v1.md.

/// One kind's current label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoticeLabel {
    pub kind: &'static str,
    pub label: &'static str,
    /// Anchored title recorded by existing notice producers.
    pub stored_title: &'static str,
}

/// The sole kind-to-label table for summary rendering and history recognition.
pub const NOTICE_LABELS: &[NoticeLabel] = &[
    NoticeLabel {
        kind: "mailbox",
        label: "Mailbox",
        stored_title: "Mailbox",
    },
    NoticeLabel {
        kind: "mailbox.wake",
        label: "Mailbox",
        stored_title: "Mailbox wake",
    },
    NoticeLabel {
        kind: "mailbox.read",
        label: "Mailbox read",
        stored_title: "Mailbox read",
    },
    NoticeLabel {
        kind: "mailbox.alert",
        label: "Mailbox alert",
        stored_title: "Mailbox alert",
    },
    NoticeLabel {
        kind: "mailbox.bounce",
        label: "Mailbox bounce",
        stored_title: "Mailbox bounce",
    },
    NoticeLabel {
        kind: "schedule.patrol",
        label: "Patrol",
        stored_title: "Patrol",
    },
    NoticeLabel {
        kind: "schedule.pulse",
        label: "Pulse",
        stored_title: "Pulse",
    },
    NoticeLabel {
        kind: "schedule.tick",
        label: "Schedule",
        stored_title: "Schedule",
    },
    NoticeLabel {
        kind: "compact.reminder",
        label: "Compact reminder",
        stored_title: "Compact reminder",
    },
    NoticeLabel {
        kind: "compact.resume",
        label: "Compact resume",
        stored_title: "Compact resume",
    },
    NoticeLabel {
        kind: "runtime.resume",
        label: "Resume",
        stored_title: "Resume",
    },
    NoticeLabel {
        kind: "context.continue",
        label: "Continue",
        stored_title: "Continue",
    },
    NoticeLabel {
        kind: "dispatch.bootstrap",
        label: "Dispatch start",
        stored_title: "Dispatch start",
    },
    NoticeLabel {
        kind: "dispatch.role",
        label: "Role dispatch",
        stored_title: "Role dispatch",
    },
    NoticeLabel {
        kind: "session.handover",
        label: "Handover",
        stored_title: "Handover",
    },
    NoticeLabel {
        kind: "monitoring.service",
        label: "Monitoring",
        stored_title: "Monitoring",
    },
    NoticeLabel {
        kind: "monitoring.mirror",
        label: "Monitoring mirror",
        stored_title: "Monitoring mirror",
    },
    NoticeLabel {
        kind: "monitoring.delivery",
        label: "Monitoring delivery",
        stored_title: "Monitoring delivery",
    },
    NoticeLabel {
        kind: "role.restart",
        label: "Role restart",
        stored_title: "Role restart",
    },
    NoticeLabel {
        kind: "role.unavailable",
        label: "Role unavailable",
        stored_title: "Role unavailable",
    },
];

/// Unknown kinds keep their exact token; namespace prefixes have no meaning.
pub fn label_for(kind: &str) -> &str {
    NOTICE_LABELS
        .iter()
        .find(|entry| entry.kind == kind)
        .map_or(kind, |entry| entry.label)
}
