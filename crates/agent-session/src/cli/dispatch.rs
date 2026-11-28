//! Root CLI dispatch, separate from runtime implementation.
use crate::*;

pub(crate) fn parsed(cli: Cli, attested_create_token: Option<String>) -> i32 {
    if let Some(code) = cli::operator_rules::prepare(&cli.command) {
        return code;
    }
    if let Command::Completion(args) = cli.command {
        return completion::run(args.shell);
    }
    let format = coordination_routing::command_format(&cli.command);
    let context = match CliContext::resolve(cli.state_dir, cli.host) {
        Ok(context) => context,
        Err(err) => {
            return render_error(
                coordination_routing::command_name(&cli.command).unwrap_or("error"),
                format,
                err,
            );
        }
    };
    if matches!(
        cli.command,
        Command::Account(_) | Command::Resume(_) | Command::Restart(_) | Command::AutoResume(_)
    ) {
        account_broker::adopt_serve_brokers(&context.state_dir);
    }
    run(&context, cli.command, attested_create_token)
}

fn run(context: &CliContext, command: Command, attested_create_token: Option<String>) -> i32 {
    match command {
        Command::AuthAlert(args) => cli::auth_alert::run(context, args),
        Command::Drafts(args) => cli::drafts::run(context, args),
        Command::Start(args) => run_start(context, *args),
        Command::Run(args) => run_one_shot(context, args),
        Command::List(args) => session_views::run_list(context, args),
        Command::Previews(args) => serve::run_previews(context, args),
        Command::Board(args) => board::run(context, args),
        Command::Show(args) => session_model::refresh::run_show(context, args),
        Command::Refresh(args) => session_model::refresh::run(context, args),
        Command::Title(args) => session_title::run(context, args),
        Command::AutoResume(args) => auto_resume::cli::run(context, args),
        Command::Attach(args) => run_attach(context, args),
        Command::Logs(args) => run_logs(context, args),
        Command::Diagnose(args) => diagnose::run_diagnose(context, args),
        Command::Send(args) => send::run(context, args),
        Command::Attempt(args) => send::attempt::run(context, args),
        Command::Permission(args) => cli::permission::run(context, args),
        Command::Notice(args) => notice::run(context, args),
        Command::NoticeStore(args) => send::claude_notice::store::audit::run(context, args),
        Command::Glance(args) => glance::run(context, args),
        Command::PrepareShutdown(args) => {
            runtime_environment::boot_recovery::run_prepare(context, args)
        }
        Command::Resume(args) => run_resume(context, args),
        Command::ResumeMove(args) => cli::resume_move::run(context, args),
        Command::Restart(args) => restart::run(context, args),
        Command::Clear(args) => conversation::run(
            context,
            args.conversation,
            conversation::Operation::Clear {
                allow_unread_mail: args.allow_unread_mail,
            },
        ),
        Command::Rebind(args) => conversation::run(context, args, conversation::Operation::Rebind),
        Command::Account(args) => session_account::run_account(context, args),
        Command::Activity(args) => run_activity(context, args),
        Command::WorkContext(args) => coordination::run_work_context(context, args),
        Command::Broker(args) => coordination::run_broker(context, args),
        Command::Readiness(args) => coordination::readiness::run(context, args),
        Command::Delegation(args) => coordination::delegation::run(context, args),
        Command::Message(args) => coordination::run_message(context, args),
        Command::Metadata(args) => metadata::run_metadata(context, args),
        Command::Lineage(args) => lineage::run_lineage(context, args),
        Command::Work(args) => lineage::run_work(context, args),
        Command::Workspace(args) => origin::run(context, args),
        Command::Serve(args) => serve::run_serve(context, args, attested_create_token),
        Command::CodexAppServerProxy(args) => codex_app_server::run_proxy(context, args),
        Command::Delete(args) => run_delete(context, args),
        Command::Tmp(args) => runtime_environment::session_tmp::run(context, args),
        Command::Completion(_) | Command::Rules(_) => unreachable!("handled earlier"),
    }
}
