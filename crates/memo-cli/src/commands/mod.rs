mod add;
mod apply;
mod delete;
mod fetch;
mod list;
mod report;
mod search;
mod update;

use crate::cli::{Cli, ItemState, MemoCommand, OutputMode, SearchMatch};
use crate::errors::AppError;
use crate::storage::Storage;
use crate::storage::repository::QueryState;

pub fn run(cli: &Cli, output_mode: OutputMode) -> Result<(), AppError> {
    let storage = Storage::new(cli.db.clone());
    storage.init()?;

    match &cli.command {
        MemoCommand::Add(args) => add::run(&storage, args, output_mode),
        MemoCommand::Update(args) => update::run(&storage, args, output_mode),
        MemoCommand::Delete(args) => delete::run(&storage, args, output_mode),
        MemoCommand::List(args) => list::run(
            &storage,
            output_mode,
            to_query_state(args.state),
            args.limit,
            args.offset,
        ),
        MemoCommand::Search(args) => search::run(
            &storage,
            output_mode,
            to_query_state(args.state),
            &args.query,
            &args.fields,
            to_search_match_mode(args.match_mode),
            args.limit,
        ),
        MemoCommand::Report(args) => report::run(&storage, output_mode, args),
        MemoCommand::Fetch(args) => {
            fetch::run(&storage, output_mode, args.limit, args.cursor.as_deref())
        }
        MemoCommand::Apply(args) => apply::run(&storage, output_mode, args),
        MemoCommand::Completion(_) => Ok(()),
    }
}

fn to_query_state(state: ItemState) -> QueryState {
    match state {
        ItemState::All => QueryState::All,
        ItemState::Pending => QueryState::Pending,
        ItemState::Enriched => QueryState::Enriched,
    }
}

fn to_search_match_mode(mode: SearchMatch) -> crate::storage::search::SearchMatchMode {
    match mode {
        SearchMatch::Fts => crate::storage::search::SearchMatchMode::Fts,
        SearchMatch::Prefix => crate::storage::search::SearchMatchMode::Prefix,
        SearchMatch::Contains => crate::storage::search::SearchMatchMode::Contains,
    }
}
