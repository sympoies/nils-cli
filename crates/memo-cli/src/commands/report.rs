use serde_json::json;

use crate::cli::{OutputMode, ReportPeriod};
use crate::errors::AppError;
use crate::output::{emit_json_result, text};
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

    text::print_report(&summary);

    Ok(())
}
