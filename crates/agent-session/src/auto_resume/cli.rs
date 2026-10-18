//! Local access to the same durable recovery controls used by serve.

use crate::cli::AutoResumeArgs;
use crate::{CliContext, load_session_record, render_error, render_single_success};

pub(crate) fn run(context: &CliContext, args: AutoResumeArgs) -> i32 {
    let result = if args.enable || args.disable {
        super::set_enabled_with_policy(
            context,
            &args.id,
            args.enable,
            args.policy
                .as_deref()
                .unwrap_or(super::WAIT_FOR_RESET_POLICY),
            &jiff::Timestamp::now().to_string(),
        )
    } else {
        load_session_record(context, &args.id)
            .map(|record| super::view_for_record(context, &record))
    };
    match result {
        Ok(view) => render_single_success("auto-resume", args.format, &view, |view| {
            format!(
                "supported: {}\nenabled: {}\npolicy: {}\nstate: {}\nattempts: {}",
                view.supported, view.enabled, view.recovery_policy, view.state, view.attempts
            )
        }),
        Err(err) => render_error("auto-resume", args.format, err),
    }
}
