use serde_json::json;

use crate::cli::{OutputMode, ReportPeriod};
use crate::errors::AppError;
use crate::output::emit_json_result;
use crate::storage::Storage;
use crate::storage::search::{self, ReportPeriod as StorageReportPeriod};

pub fn run(
    storage: &Storage,
    output_mode: OutputMode,
    period: ReportPeriod,
) -> Result<(), AppError> {
    let storage_period = match period {
        ReportPeriod::Week => StorageReportPeriod::Week,
        ReportPeriod::Month => StorageReportPeriod::Month,
    };

    let summary = storage.with_connection(|conn| search::report_summary(conn, storage_period))?;

    if output_mode.is_json() {
        return emit_json_result(
            "memo-cli.report.v1",
            "memo-cli report",
            json!({
                "period": summary.period,
                "range": summary.range,
                "totals": summary.totals,
                "top_categories": summary.top_categories,
                "top_tags": summary.top_tags,
            }),
        );
    }

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
        for item in summary.top_categories {
            println!("  - {} ({})", item.name, item.count);
        }
    }

    if !summary.top_tags.is_empty() {
        println!("top tags:");
        for item in summary.top_tags {
            println!("  - {} ({})", item.name, item.count);
        }
    }

    Ok(())
}
