use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::process;

mod commit;
mod git;
mod print;
mod render;

#[derive(Parser)]
#[command(
    name = "git-scope",
    disable_help_flag = true,
    disable_help_subcommand = true
)]
struct Cli {
    /// Disable ANSI colors (also via NO_COLOR)
    #[arg(long, global = true)]
    no_color: bool,

    /// Display help message for git-scope
    #[arg(short = 'h', long = "help", global = true)]
    help: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Show files tracked by Git (prefix filter optional)
    Tracked {
        /// Print the contents of each file
        #[arg(short = 'p', long = "print")]
        print: bool,
        /// Optional path prefixes to filter tracked files
        #[arg(value_name = "prefix", num_args = 0..)]
        prefixes: Vec<String>,
    },
    /// Show files staged for commit
    Staged {
        /// Print the contents of each file (index)
        #[arg(short = 'p', long = "print")]
        print: bool,
    },
    /// Show modified files not yet staged
    Unstaged {
        /// Print the contents of each file (worktree)
        #[arg(short = 'p', long = "print")]
        print: bool,
    },
    /// Show all changes (staged + unstaged)
    All {
        /// Print the contents of each file
        #[arg(short = 'p', long = "print")]
        print: bool,
    },
    /// Show untracked files
    Untracked {
        /// Print the contents of each file (worktree)
        #[arg(short = 'p', long = "print")]
        print: bool,
    },
    /// Show commit details (use -p to print content)
    Commit {
        /// Print file contents for the commit file list
        #[arg(short = 'p', long = "print")]
        print: bool,
        /// For merge commits: show diff against parent <n>
        #[arg(long = "parent", short = 'P')]
        parent: Option<String>,
        /// Commit-ish (hash, HEAD, etc.)
        commit: String,
    },
    /// Display help message for git-scope
    Help,
}

fn print_help() {
    println!("Usage: git-scope <command> [args]");
    println!();
    println!("Commands:");
    println!(
        "  {:<16}  Show files tracked by Git (prefix filter optional)",
        "tracked"
    );
    println!("  {:<16}  Show files staged for commit", "staged");
    println!("  {:<16}  Show modified files not yet staged", "unstaged");
    println!("  {:<16}  Show all changes (staged and unstaged)", "all");
    println!("  {:<16}  Show untracked files", "untracked");
    println!(
        "  {:<16}  Show commit details (use -p to print content)",
        "commit <id>"
    );
    println!();
    println!("Options:");
    println!(
        "  {:<16}  Print file contents where applicable (e.g., commit)",
        "-p, --print"
    );
    println!(
        "  {:<16}  Disable ANSI colors (also via NO_COLOR)",
        "--no-color"
    );
}

fn main() {
    if let Err(err) = run() {
        eprintln!("{err:#}");
        process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    let help_requested = cli.help || matches!(cli.command, Some(Command::Help));
    if help_requested {
        print_help();
        return Ok(());
    }

    if !git::is_git_repo() {
        println!("⚠️ Not a Git repository. Run this command inside a Git project.");
        process::exit(1);
    }

    let no_color = cli.no_color || std::env::var_os("NO_COLOR").is_some();
    let progress_opt_in = git_scope_progress_opt_in();

    match cli.command.unwrap_or(Command::Help) {
        Command::Tracked { print, prefixes } => {
            let lines = git::collect_tracked(&prefixes)?;
            render::render_with_type(
                &lines,
                no_color,
                render::PrintMode::Worktree,
                print,
                progress_opt_in,
            )?;
        }
        Command::Staged { print } => {
            let lines = git::collect_staged()?;
            render::render_with_type(
                &lines,
                no_color,
                render::PrintMode::Index,
                print,
                progress_opt_in,
            )?;
        }
        Command::Unstaged { print } => {
            let lines = git::collect_unstaged()?;
            render::render_with_type(
                &lines,
                no_color,
                render::PrintMode::Worktree,
                print,
                progress_opt_in,
            )?;
        }
        Command::All { print } => {
            let (combined, staged, unstaged) = git::collect_all()?;
            let files = render::render_with_type(
                &combined,
                no_color,
                render::PrintMode::Worktree,
                false,
                progress_opt_in,
            )?;
            if print {
                render::print_all_files(&files, &staged, &unstaged, progress_opt_in)?;
            }
        }
        Command::Untracked { print } => {
            let lines = git::collect_untracked()?;
            render::render_with_type(
                &lines,
                no_color,
                render::PrintMode::Worktree,
                print,
                progress_opt_in,
            )?;
        }
        Command::Commit {
            print,
            parent,
            commit,
        } => {
            commit::render_commit(&commit, parent.as_deref(), no_color, print, progress_opt_in)
                .with_context(|| format!("git-scope commit {commit}"))?;
        }
        Command::Help => {
            print_help();
        }
    }

    Ok(())
}

fn git_scope_progress_opt_in() -> bool {
    let Some(value) = std::env::var_os("GIT_SCOPE_PROGRESS") else {
        return false;
    };
    let value = value.to_string_lossy();
    let normalized = value.trim().to_ascii_lowercase();
    matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
}
