use anyhow::{Context, Result};
use chrono::{Datelike, Duration, Local, NaiveDate};
use std::env;
use std::io::IsTerminal;
use std::process::{Command, Stdio};

use nils_term::progress::{Progress, ProgressFinish, ProgressOptions};

const SEPARATOR: &str =
    "----------------------------------------------------------------------------------------------------------------------------------------";

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() || is_help(&args[0]) {
        print_help();
        return 0;
    }

    let cmd = args[0].as_str();
    match cmd {
        "all" => {
            if let Err(msg) = require_git() {
                println!("{msg}");
                return 1;
            }
            print_header("all commits");
            summary(None, None)
        }
        "today" => {
            if let Err(msg) = require_git() {
                println!("{msg}");
                return 1;
            }
            let today = format_date(Local::now().date_naive());
            print_header(&format!("today: {today}"));
            summary(Some(&today), Some(&today))
        }
        "yesterday" => {
            if let Err(msg) = require_git() {
                println!("{msg}");
                return 1;
            }
            let today = Local::now().date_naive();
            let yesterday = format_date(today - Duration::days(1));
            print_header(&format!("yesterday: {yesterday}"));
            summary(Some(&yesterday), Some(&yesterday))
        }
        "this-month" => {
            if let Err(msg) = require_git() {
                println!("{msg}");
                return 1;
            }
            let today = Local::now().date_naive();
            let start = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today);
            let start = format_date(start);
            let end = format_date(today);
            print_header(&format!("this month: {start} to {end}"));
            summary(Some(&start), Some(&end))
        }
        "last-month" => {
            if let Err(msg) = require_git() {
                println!("{msg}");
                return 1;
            }
            let today = Local::now().date_naive();
            let first_this_month =
                NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today);
            let end_last_month = first_this_month - Duration::days(1);
            let start_last_month =
                NaiveDate::from_ymd_opt(end_last_month.year(), end_last_month.month(), 1)
                    .unwrap_or(end_last_month);
            let start = format_date(start_last_month);
            let end = format_date(end_last_month);
            print_header(&format!("last month: {start} to {end}"));
            summary(Some(&start), Some(&end))
        }
        "this-week" => {
            if let Err(msg) = require_git() {
                println!("{msg}");
                return 1;
            }
            let today = Local::now().date_naive();
            let weekday = today.weekday().number_from_monday() as i64;
            let start = format_date(today - Duration::days(weekday - 1));
            let end = format_date(today + Duration::days(7 - weekday));
            print_header(&format!("this week: {start} to {end}"));
            summary(Some(&start), Some(&end))
        }
        "last-week" => {
            if let Err(msg) = require_git() {
                println!("{msg}");
                return 1;
            }
            let today = Local::now().date_naive();
            let weekday = today.weekday().number_from_monday() as i64;
            let end = format_date(today - Duration::days(weekday));
            let start = format_date((today - Duration::days(weekday)) - Duration::days(6));
            print_header(&format!("last week: {start} to {end}"));
            summary(Some(&start), Some(&end))
        }
        _ => {
            if args.len() >= 2 {
                if let Err(msg) = require_git() {
                    println!("{msg}");
                    return 1;
                }
                summary(Some(&args[0]), Some(&args[1]))
            } else {
                println!("❌ Invalid usage. Try: git-summary help");
                1
            }
        }
    }
}

fn is_help(arg: &str) -> bool {
    matches!(arg, "help" | "--help" | "-h")
}

fn print_help() {
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

fn print_header(label: &str) {
    println!();
    println!("📅 Git summary for {label}");
    println!();
}

fn require_git() -> Result<(), &'static str> {
    if Command::new("git")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_err()
    {
        return Err("❗ git is required but was not found in PATH.");
    }

    let status = Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    if !status.map(|s| s.success()).unwrap_or(false) {
        return Err("⚠️ Not a Git repository. Run this command inside a Git project.");
    }

    Ok(())
}

fn summary(since: Option<&str>, until: Option<&str>) -> i32 {
    if (since.is_some() && until.is_none()) || (since.is_none() && until.is_some()) {
        println!("❌ Please provide both start and end dates (YYYY-MM-DD).");
        return 1;
    }

    if let Some(value) = since {
        if let Err(msg) = validate_date(value) {
            println!("{msg}");
            return 1;
        }
    }
    if let Some(value) = until {
        if let Err(msg) = validate_date(value) {
            println!("{msg}");
            return 1;
        }
    }

    if let (Some(start), Some(end)) = (since, until) {
        if start > end {
            println!("❌ Start date must be on or before end date.");
            return 1;
        }
    }

    let log_args = match (since, until) {
        (Some(start), Some(end)) => build_range_args(start, end),
        _ => vec!["--no-merges".to_string()],
    };

    match render_summary(&log_args) {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("{err:#}");
            1
        }
    }
}

fn validate_date(input: &str) -> Result<(), String> {
    if input.is_empty() {
        return Err("❌ Missing date value.".to_string());
    }
    if !is_date_format(input) {
        return Err(format!(
            "❌ Invalid date format: {input} (expected YYYY-MM-DD)."
        ));
    }
    if NaiveDate::parse_from_str(input, "%Y-%m-%d").is_err() {
        return Err(format!("❌ Invalid date value: {input}."));
    }
    Ok(())
}

fn is_date_format(input: &str) -> bool {
    let bytes = input.as_bytes();
    if bytes.len() != 10 {
        return false;
    }
    if bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    bytes
        .iter()
        .enumerate()
        .filter(|(idx, _)| *idx != 4 && *idx != 7)
        .all(|(_, b)| b.is_ascii_digit())
}

fn build_range_args(since: &str, until: &str) -> Vec<String> {
    let tz = Local::now().format("%z").to_string();
    let since_bound = format!("{since} 00:00:00 {tz}");
    let until_bound = format!("{until} 23:59:59 {tz}");
    vec![
        format!("--since={since_bound}"),
        format!("--until={until_bound}"),
        "--no-merges".to_string(),
    ]
}

fn render_summary(log_args: &[String]) -> Result<()> {
    let authors = collect_authors(log_args)?;
    let progress = if !authors.is_empty() && std::io::stderr().is_terminal() {
        Some(Progress::new(
            authors.len() as u64,
            ProgressOptions::default().with_finish(ProgressFinish::Clear),
        ))
    } else {
        None
    };

    let mut rows = Vec::new();
    for (idx, author) in authors.iter().enumerate() {
        if let Some(p) = &progress {
            p.set_message(author.to_string());
        }

        rows.push(collect_author_row(author, log_args)?);

        if let Some(p) = &progress {
            p.set_position((idx + 1) as u64);
        }
    }

    rows.sort_by(|a, b| b.net.cmp(&a.net).then_with(|| a.line.cmp(&b.line)));

    if let Some(p) = progress {
        p.finish_and_clear();
    }

    println!(
        "{:<25} {:<40} {:>8} {:>8} {:>8} {:>8} {:>12} {:>12}",
        "Name", "Email", "Added", "Deleted", "Net", "Commits", "First", "Last"
    );
    println!("{SEPARATOR}");

    for row in rows {
        println!("{}", row.line);
    }

    Ok(())
}

fn collect_authors(log_args: &[String]) -> Result<Vec<String>> {
    let mut args = vec!["log".to_string()];
    args.extend(log_args.iter().cloned());
    args.push("--pretty=format:%an <%ae>".to_string());

    let output = run_git(&args)?;
    let mut authors = output
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .map(|line| line.to_string())
        .collect::<Vec<_>>();
    authors.sort();
    authors.dedup();
    Ok(authors)
}

fn collect_author_row(author: &str, log_args: &[String]) -> Result<Row> {
    let (name, email) = split_author(author);
    let short_email = truncate_email(&email);

    let mut args = vec!["log".to_string()];
    args.extend(log_args.iter().cloned());
    if !email.is_empty() {
        args.push(format!("--author={email}"));
    }
    args.push("--pretty=format:%cd".to_string());
    args.push("--date=short".to_string());
    args.push("--numstat".to_string());

    let log = run_git(&args)?;

    let (commits, first_commit, last_commit) = parse_commit_dates(&log);
    let (added, deleted) = parse_numstat_totals(&log);
    let net = added - deleted;

    let line = format!(
        "{:<25} {:<40} {:>8} {:>8} {:>8} {:>8} {:>12} {:>12}",
        name, short_email, added, deleted, net, commits, first_commit, last_commit
    );

    Ok(Row { net, line })
}

fn split_author(author: &str) -> (String, String) {
    if let (Some(start), Some(end)) = (author.find('<'), author.rfind('>')) {
        if start < end {
            let name = author[..start].trim().to_string();
            let email = author[start + 1..end].trim().to_string();
            return (name, email);
        }
    }
    (author.trim().to_string(), String::new())
}

fn truncate_email(email: &str) -> String {
    email.chars().take(40).collect::<String>()
}

fn parse_commit_dates(log: &str) -> (i64, String, String) {
    let mut commits = 0i64;
    let mut first_commit = String::new();
    let mut last_commit = String::new();

    for line in log.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() == 1 {
            commits += 1;
            if last_commit.is_empty() {
                last_commit = parts[0].to_string();
            }
            first_commit = parts[0].to_string();
        }
    }

    (commits, first_commit, last_commit)
}

fn parse_numstat_totals(log: &str) -> (i64, i64) {
    let mut added = 0i64;
    let mut deleted = 0i64;

    for line in log.lines() {
        if is_lockfile_line(line) {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() == 3 {
            added += parts[0].parse::<i64>().unwrap_or(0);
            deleted += parts[1].parse::<i64>().unwrap_or(0);
        }
    }

    (added, deleted)
}

fn is_lockfile_line(line: &str) -> bool {
    let trimmed = line.trim_end();
    trimmed.ends_with("yarn.lock")
        || trimmed.ends_with("package-lock.json")
        || trimmed.ends_with("pnpm-lock.yaml")
        || trimmed.ends_with(".lock")
}

fn run_git(args: &[String]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("git {args:?}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git {args:?} failed: {stderr}");
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

struct Row {
    net: i64,
    line: String,
}

fn format_date(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}
