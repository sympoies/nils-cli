pub const NOT_GIT_REPO: &str = "❗ Not a Git repository. Run this command inside a Git project.";
pub const UNKNOWN_COMMAND_HINT: &str = "Run 'git-lock help' for usage.";
pub const COPY_USAGE: &str = "❗ Usage: git-lock-copy <source-label> <target-label>";
pub const TARGET_LABEL_MISSING: &str = "❗ Target label is missing";
pub const NO_GIT_LOCKS_FOUND: &str = "❌ No git-locks found";
pub const DIFF_USAGE: &str = "❗ Usage: git-lock diff <label1> <label2> [--no-color]";
pub const TAG_USAGE: &str =
    "❗ Usage: git-lock tag <git-lock-label> <tag-name> [-m <tag-message>] [--push]";

pub fn unknown_command(cmd: &str) -> String {
    format!("❗ Unknown command: '{cmd}'")
}

pub fn print_help() {
    println!("Usage: git-lock <command> [args]");
    println!();
    println!("Commands:");
    println!(
        "  {:<16}  Save commit hash to lock",
        "lock [label] [note] [commit]"
    );
    println!("  {:<16}  Reset to a saved commit", "unlock [label]");
    println!("  {:<16}  Show all locks for repo", "list");
    println!("  {:<16}  Duplicate a lock label", "copy <from> <to>");
    println!("  {:<16}  Remove a lock", "delete [label]");
    println!(
        "  {:<16}  Compare commits between two locks",
        "diff <l1> <l2> [--no-color]"
    );
    println!(
        "  {:<16}  Create git tag from a lock",
        "tag <label> <tag> [-m msg]"
    );
    println!();
}
