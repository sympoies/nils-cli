//! Shared validation argument preparation.

pub(super) fn validate_args_with_parent_fallbacks(
    parent: &crate::cli::PrReviewArgs,
    mut validate_args: crate::cli::PrReviewValidateArgs,
) -> crate::cli::PrReviewValidateArgs {
    if validate_args.id.is_none() {
        validate_args.id = parent.id;
    }
    if validate_args.comment.is_none() && validate_args.comment_file.is_none() {
        validate_args.comment = parent.comment.clone();
        validate_args.comment_file = parent.comment_file.clone();
    }
    if validate_args.thread_file.is_none() {
        validate_args.thread_file = parent.thread_file.clone();
    }
    if !validate_args.specialist_report {
        validate_args.specialist_report = parent.specialist_report;
    }
    validate_args
}
