mod confirm;
mod defs;
mod directory;
mod file;
mod fzf;
mod git_branch;
mod git_checkout;
mod git_commit;
mod git_commit_select;
mod git_status;
mod git_tag;
mod history;
mod kill;
mod open;
mod port;
mod process;
mod util;

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || util::is_help(&args[0]) {
        print_help();
        return 0;
    }

    let cmd = args[0].as_str();
    let rest = &args[1..];

    match cmd {
        "help" => {
            print_help();
            0
        }
        "file" => file::run(rest),
        "directory" => directory::run(rest),
        "git-status" => git_status::run(rest),
        "git-commit" => git_commit::run(rest),
        "git-checkout" => git_checkout::run(rest),
        "git-branch" => git_branch::run(rest),
        "git-tag" => git_tag::run(rest),
        "process" => process::run(rest),
        "port" => port::run(rest),
        "history" => history::run(rest),
        "env" => defs::run_env(rest),
        "alias" => defs::run_alias(rest),
        "function" => defs::run_function(rest),
        "def" => defs::run_def(rest),
        _ => {
            println!("❗ Unknown command: {cmd}");
            println!("Run 'fzf-cli help' for usage.");
            1
        }
    }
}

fn print_help() {
    println!("Usage: fzf-cli <command> [args]");
    println!();
    println!("Commands:");
    println!("  {:<16}  Search and preview text files", "file");
    println!(
        "  {:<16}  Search directories and cd into selection",
        "directory"
    );
    println!("  {:<16}  Interactive git status viewer", "git-status");
    println!(
        "  {:<16}  Browse commits and open changed files in editor",
        "git-commit"
    );
    println!(
        "  {:<16}  Pick and checkout a previous commit",
        "git-checkout"
    );
    println!(
        "  {:<16}  Browse and checkout branches interactively",
        "git-branch"
    );
    println!(
        "  {:<16}  Browse and checkout tags interactively",
        "git-tag"
    );
    println!(
        "  {:<16}  Browse and kill running processes (confirm before kill)",
        "process"
    );
    println!(
        "  {:<16}  Browse listening ports and owners (confirm before kill)",
        "port"
    );
    println!("  {:<16}  Search and execute command history", "history");
    println!("  {:<16}  Browse environment variables", "env");
    println!("  {:<16}  Browse shell aliases", "alias");
    println!("  {:<16}  Browse defined shell functions", "function");
    println!(
        "  {:<16}  Browse all definitions (env, alias, functions)",
        "def"
    );
    println!();
}
