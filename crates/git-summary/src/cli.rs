pub fn print_help() {
    println!("Usage: git-summary <command> [args]");
    println!();
    println!("Commands:");
    println!("  {:<16}  Entire history", "all");
    println!("  {:<16}  Today only", "today");
    println!("  {:<16}  Yesterday only", "yesterday");
    println!("  {:<16}  1st to today", "this-month");
    println!("  {:<16}  1st to end of last month", "last-month");
    println!("  {:<16}  This Mon–Sun", "this-week");
    println!("  {:<16}  Last Mon–Sun", "last-week");
    println!("  {:<16}  Custom date range (YYYY-MM-DD)", "<from> <to>");
    println!();
}

pub fn print_header(label: &str) {
    println!();
    println!("📅 Git summary for {label}");
    println!();
}
