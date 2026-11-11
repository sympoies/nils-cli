use super::*;

mod mailbox_controller {
    include!("mailbox_controller.rs");
}

mod draft_guard {
    include!("mailbox_draft.rs");
}

mod notice {
    include!("mailbox_notice.rs");
}

mod fixture {
    include!("mailbox_fixture.rs");
}
mod steer {
    include!("mailbox_steer.rs");
}

mod wire {
    include!("mailbox_wire.rs");
}
