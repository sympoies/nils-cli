//! Issue subcommands and dispatch.

use super::*;

/// `issue` subtree.
#[derive(Subcommand, Debug)]
pub enum IssueCommand {
    /// Open a new issue.
    Create(IssueCreateArgs),
    /// Fetch a single issue.
    View(IssueViewArgs),
    /// Read GitHub issue closure and reopen events.
    Events(IssueEventsArgs),
    /// List issues filtered by state / labels / author / assignee.
    List(IssueListArgs),
    /// Mutate an issue.
    Edit(IssueEditArgs),
    /// Append a comment to an issue.
    Comment(IssueCommentArgs),
    /// Edit an existing issue comment.
    CommentEdit(TimelineCommentEditArgs),
    /// Close an issue.
    Close(IssueCloseArgs),
    /// Reopen a closed issue.
    Reopen {
        /// Numeric id.
        id: u64,
    },
    /// Check and maintain a program tracker issue's phase table.
    Tracker(IssueTrackerArgs),
}

/// Read GitHub issue closure and reopen events with actor, timestamp, and closer references.
#[derive(Args, Debug)]
pub struct IssueEventsArgs {
    /// Numeric GitHub issue number.
    #[arg(value_parser = clap::value_parser!(i32).range(1..))]
    pub id: i32,
}

pub(super) fn run(
    global: &GlobalFlags,
    command: IssueCommand,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    match command {
        IssueCommand::Create(args) => ops::issue_create::run(global, args, format),
        IssueCommand::View(args) => ops::issue_view::run(global, args, format),
        IssueCommand::Events(args) => ops::issue_events::run(global, args, format),
        IssueCommand::List(args) => ops::issue_list::run(global, args, format),
        IssueCommand::Edit(args) => ops::issue_edit::run(global, args, format),
        IssueCommand::Comment(args) => ops::issue_comment::run(global, args, format),
        IssueCommand::CommentEdit(args) => ops::comment_edit::run(global, args, format, false),
        IssueCommand::Close(args) => ops::issue_close::run(global, args.id, args.reason, format),
        IssueCommand::Reopen { id } => ops::issue_reopen::run(global, id, format),
        IssueCommand::Tracker(args) => match args.command {
            IssueTrackerCommand::Lint(args) => ops::issue_tracker_lint::run(global, args, format),
            IssueTrackerCommand::Graph(args) => ops::issue_tracker_graph::run(global, args, format),
            IssueTrackerCommand::Tick(args) => ops::issue_tracker_tick::run(global, args, format),
            IssueTrackerCommand::Show(args) => ops::issue_tracker_show::run(global, args, format),
        },
    }
}
