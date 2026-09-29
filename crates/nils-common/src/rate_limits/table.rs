//! The all-accounts table printed by `--all` and `--async` text output.

use std::collections::BTreeSet;

use super::schema::LOCAL_DATETIME_WITH_OFFSET;
use super::values::format_until_epoch_compact;
use crate::rate_limits_ansi as ansi;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowState {
    /// No value available (genuine fetch failure with no cache to fall back to).
    Missing,
    /// Fresh live (or within-TTL cache) values.
    Filled,
    /// Last-known values served from cache past the freshness TTL.
    Stale,
    /// Backend reported no active rate-limit window and no cache was available.
    NoWindow,
}

pub struct Row {
    pub name: String,
    pub window_label: String,
    pub non_weekly_remaining: i64,
    pub non_weekly_reset_epoch: Option<i64>,
    pub weekly_remaining: i64,
    pub weekly_reset_epoch: Option<i64>,
    pub weekly_reset_iso: String,
    pub reset_credits_available: Option<i64>,
    pub state: RowState,
}

impl Row {
    pub fn empty(name: String) -> Self {
        Self {
            name,
            window_label: String::new(),
            non_weekly_remaining: -1,
            non_weekly_reset_epoch: None,
            weekly_remaining: -1,
            weekly_reset_epoch: None,
            weekly_reset_iso: String::new(),
            reset_credits_available: None,
            state: RowState::Missing,
        }
    }

    fn sort_key(&self) -> (i32, i64, String) {
        if let Some(epoch) = self.weekly_reset_epoch {
            (0, epoch, self.name.clone())
        } else {
            (1, i64::MAX, self.name.clone())
        }
    }
}

struct TableDisplayRow {
    is_current: bool,
    name: String,
    non_weekly: String,
    non_weekly_left: String,
    weekly: String,
    weekly_left: String,
    reset: String,
    resets: String,
}

/// Inputs of one table render besides the rows.
pub struct TableContext<'a> {
    /// Heading text after the traffic-light glyph.
    pub title: &'a str,
    /// Distinct non-weekly labels across rows.
    pub window_labels: &'a BTreeSet<String>,
    /// Row name of the active login, highlighted on a terminal.
    pub current_name: Option<&'a str>,
    /// `--watch` footer time.
    pub update_time: Option<&'a str>,
    pub now_epoch: i64,
    /// Formats an epoch in local time with a strftime layout.
    pub format_local: &'a dyn Fn(i64, &str) -> Option<String>,
}

/// Renders the table, one `\n`-terminated line per printed line.
pub fn render_all_accounts_table(mut rows: Vec<Row>, context: &TableContext<'_>) -> String {
    let mut out = String::new();
    out.push_str(&format!("\n🚦 {}\n\n", context.title));

    let mut non_weekly_header = "Non-weekly".to_string();
    let multiple_labels = context.window_labels.len() != 1;
    if !multiple_labels && let Some(label) = context.window_labels.iter().next() {
        non_weekly_header = label.clone();
    }

    rows.sort_by_key(|row| row.sort_key());
    let now_epoch = context.now_epoch;
    let display_rows: Vec<_> = rows
        .into_iter()
        .map(|row| {
            // A null window reports "n/a"; a stale fallback keeps its values but is
            // marked so the reader knows they are not live.
            let no_window = row.state == RowState::NoWindow;

            let display_non_weekly = if no_window {
                "n/a".to_string()
            } else if multiple_labels && !row.window_label.is_empty() {
                if row.non_weekly_remaining >= 0 {
                    format!("{}:{}%", row.window_label, row.non_weekly_remaining)
                } else {
                    "-".to_string()
                }
            } else if row.non_weekly_remaining >= 0 {
                format!("{}%", row.non_weekly_remaining)
            } else {
                "-".to_string()
            };

            let non_weekly_left = row
                .non_weekly_reset_epoch
                .and_then(|epoch| format_until_epoch_compact(epoch, now_epoch))
                .unwrap_or_else(|| "-".to_string());
            let weekly_left = row
                .weekly_reset_epoch
                .and_then(|epoch| format_until_epoch_compact(epoch, now_epoch))
                .unwrap_or_else(|| "-".to_string());
            let mut reset_display = if no_window {
                "n/a".to_string()
            } else {
                row.weekly_reset_epoch
                    .and_then(|epoch| (context.format_local)(epoch, LOCAL_DATETIME_WITH_OFFSET))
                    .unwrap_or_else(|| "-".to_string())
            };
            if row.state == RowState::Stale {
                reset_display.push_str(" (stale)");
            }

            let weekly_display = if no_window {
                "n/a".to_string()
            } else if row.weekly_remaining >= 0 {
                format!("{}%", row.weekly_remaining)
            } else {
                "-".to_string()
            };

            TableDisplayRow {
                is_current: context.current_name == Some(row.name.as_str()),
                name: row.name,
                non_weekly: display_non_weekly,
                non_weekly_left,
                weekly: weekly_display,
                weekly_left,
                reset: reset_display,
                resets: row
                    .reset_credits_available
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_string()),
            }
        })
        .collect();

    let mut name_width = 15usize;
    let mut non_weekly_width = non_weekly_header.chars().count().max(8);
    let mut non_weekly_left_width = 7usize;
    let mut weekly_width = 8usize;
    let mut weekly_left_width = 7usize;
    let mut reset_width = 20usize;
    let mut resets_width = 6usize;
    for row in &display_rows {
        name_width = name_width.max(row.name.chars().count());
        non_weekly_width = non_weekly_width.max(row.non_weekly.chars().count());
        non_weekly_left_width = non_weekly_left_width.max(row.non_weekly_left.chars().count());
        weekly_width = weekly_width.max(row.weekly.chars().count());
        weekly_left_width = weekly_left_width.max(row.weekly_left.chars().count());
        reset_width = reset_width.max(row.reset.chars().count());
        resets_width = resets_width.max(row.resets.chars().count());
    }

    let header = format!(
        "{:<name_width$}  {:>non_weekly_width$}  {:>non_weekly_left_width$}  {:>weekly_width$}  {:>weekly_left_width$}  {:<reset_width$}  {:>resets_width$}",
        "Name", non_weekly_header, "Left", "Weekly", "Left", "Reset", "Resets"
    );
    out.push_str(&header);
    out.push('\n');
    out.push_str(&"-".repeat(header.chars().count()));
    out.push('\n');

    for row in display_rows {
        let name = ansi::format_name_cell(&row.name, name_width, row.is_current, None);
        let non_weekly = ansi::format_percent_cell(&row.non_weekly, non_weekly_width, None);
        let weekly = ansi::format_percent_cell(&row.weekly, weekly_width, None);
        out.push_str(&format!(
            "{}  {}  {:>non_weekly_left_width$}  {}  {:>weekly_left_width$}  {:<reset_width$}  {:>resets_width$}\n",
            name, non_weekly, row.non_weekly_left, weekly, row.weekly_left, row.reset, row.resets,
        ));
    }

    if let Some(update_time) = context.update_time {
        out.push('\n');
        out.push_str(&format!("Last update: {update_time}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rate_limits::test_support::utc_format;
    use nils_test_support::{EnvGuard, GlobalStateLock};
    use pretty_assertions::assert_eq;

    #[test]
    fn table_marks_stale_and_no_window_rows() {
        let lock = GlobalStateLock::new();
        let _no_color = EnvGuard::set(&lock, "NO_COLOR", "1");
        let mut filled = Row::empty("alpha".to_string());
        filled.window_label = "5h".to_string();
        filled.non_weekly_remaining = 75;
        filled.weekly_remaining = 60;
        filled.weekly_reset_epoch = Some(1_700_500_000);
        filled.state = RowState::Stale;
        let mut empty = Row::empty("beta".to_string());
        empty.state = RowState::NoWindow;
        let labels = BTreeSet::from(["5h".to_string()]);

        let rendered = render_all_accounts_table(
            vec![empty, filled],
            &TableContext {
                title: "Example rate limits for all accounts",
                window_labels: &labels,
                current_name: None,
                update_time: Some("now"),
                now_epoch: 1_800_000_000,
                format_local: &utc_format,
            },
        );

        assert_eq!(
            rendered,
            "\n🚦 Example rate limits for all accounts\n\n\
Name                   5h     Left    Weekly     Left  Reset                       Resets\n\
-----------------------------------------------------------------------------------------\n\
alpha                 75%        -       60%   0h  0m  11-20 17:06 +00:00 (stale)       -\n\
beta                  n/a        -       n/a        -  n/a                              -\n\
\nLast update: now\n"
        );
    }
}
