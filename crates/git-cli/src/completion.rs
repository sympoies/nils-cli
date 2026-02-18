use clap::{Arg, ArgAction, Command, ValueHint};
use clap_complete::{Generator, Shell, generate};
use std::io;

pub fn dispatch(shell_raw: &str, extra: &[String]) -> i32 {
    if !extra.is_empty() {
        eprintln!("git-cli: error: expected `git-cli completion <bash|zsh>`");
        return 1;
    }

    match shell_raw {
        "bash" => generate_script(Shell::Bash),
        "zsh" => generate_script(Shell::Zsh),
        other => {
            eprintln!("git-cli: error: unsupported completion shell '{other}'");
            eprintln!("usage: git-cli completion <bash|zsh>");
            1
        }
    }
}

fn generate_script<G: Generator>(generator: G) -> i32 {
    let mut command = build_command_model();
    let bin_name = command.get_name().to_string();
    generate(generator, &mut command, bin_name, &mut io::stdout());
    0
}

fn build_command_model() -> Command {
    Command::new("git-cli")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Git helper CLI")
        .disable_help_subcommand(true)
        .subcommand(build_utils_group())
        .subcommand(build_reset_group())
        .subcommand(build_commit_group())
        .subcommand(build_branch_group())
        .subcommand(build_ci_group())
        .subcommand(build_open_group())
        .subcommand(Command::new("help").about("Display help message for git-cli"))
        .subcommand(
            Command::new("completion")
                .about("Export shell completion script")
                .arg(
                    Arg::new("shell")
                        .value_name("shell")
                        .value_parser(["bash", "zsh"])
                        .required(true),
                ),
        )
}

fn build_utils_group() -> Command {
    Command::new("utils")
        .about("Utility helpers")
        .subcommand(Command::new("zip").about("Create zip archive from HEAD"))
        .subcommand(
            Command::new("copy-staged")
                .visible_alias("copy")
                .about("Copy staged diff to clipboard")
                .arg(Arg::new("stdout").long("stdout").action(ArgAction::SetTrue))
                .arg(
                    Arg::new("print")
                        .short('p')
                        .long("print")
                        .action(ArgAction::SetTrue),
                )
                .arg(Arg::new("both").long("both").action(ArgAction::SetTrue)),
        )
        .subcommand(
            Command::new("root")
                .about("Jump to git root")
                .arg(Arg::new("shell").long("shell").action(ArgAction::SetTrue)),
        )
        .subcommand(
            Command::new("commit-hash")
                .visible_alias("hash")
                .about("Resolve commit hash")
                .arg(Arg::new("ref").value_name("ref")),
        )
        .subcommand(Command::new("help").about("Display help message for utils"))
}

fn build_reset_group() -> Command {
    let count_arg = || Arg::new("count").value_name("count");

    Command::new("reset")
        .about("Reset helpers")
        .subcommand(
            Command::new("soft")
                .about("Reset to HEAD~N (soft)")
                .arg(count_arg()),
        )
        .subcommand(
            Command::new("mixed")
                .about("Reset to HEAD~N (mixed)")
                .arg(count_arg()),
        )
        .subcommand(
            Command::new("hard")
                .about("Reset to HEAD~N (hard)")
                .arg(count_arg()),
        )
        .subcommand(Command::new("undo").about("Undo last reset"))
        .subcommand(Command::new("back-head").about("Checkout HEAD@{1}"))
        .subcommand(Command::new("back-checkout").about("Return to previous branch"))
        .subcommand(
            Command::new("remote")
                .about("Reset to remote branch")
                .arg(Arg::new("ref").long("ref").value_name("ref"))
                .arg(
                    Arg::new("remote")
                        .short('r')
                        .long("remote")
                        .value_name("remote"),
                )
                .arg(
                    Arg::new("branch")
                        .short('b')
                        .long("branch")
                        .value_name("branch"),
                )
                .arg(
                    Arg::new("no-fetch")
                        .long("no-fetch")
                        .action(ArgAction::SetTrue),
                )
                .arg(Arg::new("prune").long("prune").action(ArgAction::SetTrue))
                .arg(Arg::new("clean").long("clean").action(ArgAction::SetTrue))
                .arg(
                    Arg::new("set-upstream")
                        .long("set-upstream")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("yes")
                        .short('y')
                        .long("yes")
                        .action(ArgAction::SetTrue),
                ),
        )
        .subcommand(Command::new("help").about("Display help message for reset"))
}

fn build_commit_group() -> Command {
    Command::new("commit")
        .about("Commit helpers")
        .subcommand(
            Command::new("context")
                .about("Print commit context")
                .arg(Arg::new("stdout").long("stdout").action(ArgAction::SetTrue))
                .arg(Arg::new("both").long("both").action(ArgAction::SetTrue))
                .arg(
                    Arg::new("no-color")
                        .long("no-color")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("include")
                        .long("include")
                        .value_name("glob")
                        .num_args(1..),
                ),
        )
        .subcommand(
            Command::new("context-json")
                .visible_aliases(["context_json", "contextjson", "json"])
                .about("Print commit context as JSON")
                .arg(Arg::new("stdout").long("stdout").action(ArgAction::SetTrue))
                .arg(Arg::new("both").long("both").action(ArgAction::SetTrue))
                .arg(Arg::new("pretty").long("pretty").action(ArgAction::SetTrue))
                .arg(Arg::new("bundle").long("bundle").action(ArgAction::SetTrue))
                .arg(Arg::new("out-dir").long("out-dir").value_name("path")),
        )
        .subcommand(
            Command::new("to-stash")
                .visible_alias("stash")
                .about("Create stash from commit")
                .arg(Arg::new("ref").value_name("ref")),
        )
        .subcommand(Command::new("help").about("Display help message for commit"))
}

fn build_branch_group() -> Command {
    Command::new("branch")
        .about("Branch helpers")
        .subcommand(
            Command::new("cleanup")
                .visible_alias("delete-merged")
                .about("Delete merged branches")
                .arg(Arg::new("base").short('b').long("base").value_name("base"))
                .arg(
                    Arg::new("squash")
                        .short('s')
                        .long("squash")
                        .action(ArgAction::SetTrue),
                ),
        )
        .subcommand(Command::new("help").about("Display help message for branch"))
}

fn build_ci_group() -> Command {
    Command::new("ci")
        .about("CI helpers")
        .subcommand(
            Command::new("pick")
                .about("Cherry-pick into CI branch")
                .arg(
                    Arg::new("remote")
                        .short('r')
                        .long("remote")
                        .value_name("name"),
                )
                .arg(
                    Arg::new("no-fetch")
                        .long("no-fetch")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("force")
                        .short('f')
                        .long("force")
                        .action(ArgAction::SetTrue),
                )
                .arg(Arg::new("stay").long("stay").action(ArgAction::SetTrue)),
        )
        .subcommand(Command::new("help").about("Display help message for ci"))
}

fn build_open_group() -> Command {
    Command::new("open")
        .about("Open remote pages")
        .subcommand(
            Command::new("repo")
                .about("Open repository page")
                .arg(remotes_arg()),
        )
        .subcommand(
            Command::new("branch")
                .about("Open branch tree page")
                .arg(Arg::new("ref").value_name("ref")),
        )
        .subcommand(
            Command::new("default-branch")
                .visible_alias("default")
                .about("Open default branch tree page")
                .arg(remotes_arg()),
        )
        .subcommand(
            Command::new("commit")
                .about("Open commit page")
                .arg(Arg::new("ref").value_name("ref")),
        )
        .subcommand(
            Command::new("compare")
                .about("Open compare page")
                .arg(Arg::new("from").value_name("from"))
                .arg(Arg::new("to").value_name("to")),
        )
        .subcommand(
            Command::new("pr")
                .visible_aliases(["pull-request", "mr", "merge-request"])
                .about("Open pull or merge request page")
                .arg(Arg::new("id").value_name("id")),
        )
        .subcommand(
            Command::new("pulls")
                .visible_aliases(["prs", "merge-requests", "mrs"])
                .about("Open pull or merge request list"),
        )
        .subcommand(
            Command::new("issues")
                .visible_alias("issue")
                .about("Open issues list/page")
                .arg(Arg::new("id").value_name("id")),
        )
        .subcommand(
            Command::new("actions")
                .visible_alias("action")
                .about("Open actions page")
                .arg(Arg::new("workflow").value_name("workflow")),
        )
        .subcommand(
            Command::new("releases")
                .visible_alias("release")
                .about("Open releases list/page")
                .arg(Arg::new("tag").value_name("tag")),
        )
        .subcommand(
            Command::new("tags")
                .visible_alias("tag")
                .about("Open tags list/page")
                .arg(Arg::new("tag").value_name("tag")),
        )
        .subcommand(
            Command::new("commits")
                .visible_alias("history")
                .about("Open commit history page")
                .arg(Arg::new("ref").value_name("ref")),
        )
        .subcommand(
            Command::new("file")
                .visible_alias("blob")
                .about("Open file page")
                .arg(
                    Arg::new("path")
                        .value_name("path")
                        .value_hint(ValueHint::FilePath),
                )
                .arg(Arg::new("ref").value_name("ref")),
        )
        .subcommand(
            Command::new("blame")
                .about("Open blame page")
                .arg(
                    Arg::new("path")
                        .value_name("path")
                        .value_hint(ValueHint::FilePath),
                )
                .arg(Arg::new("ref").value_name("ref")),
        )
        .subcommand(Command::new("help").about("Display help message for open"))
}

fn remotes_arg() -> Arg {
    Arg::new("remote").value_name("remote")
}
