use crate::output::format_item_id;
use crate::storage::derivations::ApplySummary;
use crate::storage::repository::{FetchItem, ListItem};
use crate::storage::search::{ReportSummary, SearchItem};

pub fn print_add(item_id: i64, created_at: &str) {
    println!("added {} at {}", format_item_id(item_id), created_at);
}

pub fn print_list(rows: &[ListItem]) {
    if rows.is_empty() {
        println!("(no items)");
        return;
    }

    println!(
        "{}\t{}\t{}\t{}",
        style_heading("item_id"),
        style_heading("created_at"),
        style_heading("state"),
        style_heading("preview")
    );
    for row in rows {
        println!(
            "{}\t{}\t{}\t{}",
            format_item_id(row.item_id),
            row.created_at,
            style_state(&row.state),
            row.text_preview
        );
    }
}

pub fn print_search(rows: &[SearchItem]) {
    if rows.is_empty() {
        println!("(no matches)");
        return;
    }

    println!(
        "{}\t{}\t{}\t{}",
        style_heading("item_id"),
        style_heading("created_at"),
        style_heading("score"),
        style_heading("preview")
    );
    for row in rows {
        println!(
            "{}\t{}\t{:.4}\t{}",
            format_item_id(row.item_id),
            row.created_at,
            row.score,
            row.preview
        );
    }
}

pub fn print_report(summary: &ReportSummary) {
    println!("report: {}", summary.period);
    println!(
        "range: {} .. {} ({})",
        summary.range.from, summary.range.to, summary.range.timezone
    );
    println!("captured: {}", summary.totals.captured);
    println!("enriched: {}", summary.totals.enriched);
    println!("pending: {}", summary.totals.pending);

    if !summary.top_categories.is_empty() {
        println!("top categories:");
        for item in &summary.top_categories {
            println!("  - {} ({})", item.name, item.count);
        }
    }

    if !summary.top_tags.is_empty() {
        println!("top tags:");
        for item in &summary.top_tags {
            println!("  - {} ({})", item.name, item.count);
        }
    }
}

pub fn print_fetch(rows: &[FetchItem]) {
    println!("pending items: {}", rows.len());
    for row in rows {
        println!(
            "{}\t{}\t{}\t{}",
            format_item_id(row.item_id),
            row.created_at,
            row.source,
            row.text
        );
    }
}

pub fn print_apply(summary: &ApplySummary) {
    println!(
        "apply payload processed={} accepted={} skipped={} failed={} dry_run={}",
        summary.processed, summary.accepted, summary.skipped, summary.failed, summary.dry_run
    );

    for item in &summary.items {
        if let Some(error) = &item.error {
            eprintln!(
                "warning: {} {}: {}",
                format_item_id(item.item_id),
                item.status,
                error.message
            );
        }
    }
}

fn style_heading(label: &str) -> String {
    if color_enabled() {
        format!("\u{1b}[1m{label}\u{1b}[0m")
    } else {
        label.to_string()
    }
}

fn style_state(state: &str) -> String {
    if !color_enabled() {
        return state.to_string();
    }

    match state {
        "pending" => format!("\u{1b}[33m{state}\u{1b}[0m"),
        "enriched" => format!("\u{1b}[32m{state}\u{1b}[0m"),
        _ => state.to_string(),
    }
}

fn color_enabled() -> bool {
    match std::env::var("NO_COLOR") {
        Ok(value) => value.trim().is_empty(),
        Err(_) => true,
    }
}
