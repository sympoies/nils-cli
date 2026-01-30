use crate::model::ItemResult;

pub fn render_report_md(
    run_id: &str,
    subcommand: &str,
    items: &[ItemResult],
    commands: &[String],
    dry_run: bool,
) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("# Image Processing Report ({run_id})"));
    lines.push(String::new());
    lines.push(format!("- Operation: `{subcommand}`"));
    lines.push(format!(
        "- Dry run: `{}`",
        if dry_run { "true" } else { "false" }
    ));
    lines.push(String::new());

    lines.push("## Commands".to_string());
    for c in commands {
        lines.push(format!("- `{c}`"));
    }
    lines.push(String::new());

    lines.push("## Results".to_string());
    for item in items {
        let status = item.status.as_str();
        let inp = item.input_path.as_str();
        let outp = item.output_path.as_deref().unwrap_or("None");
        lines.push(format!("- `{status}`: `{inp}` -> `{outp}`"));

        let in_size = item.input_info.size_bytes;
        let out_size = item.output_info.as_ref().and_then(|x| x.size_bytes);

        if let Some(bytes) = in_size {
            lines.push(format!("  - input_bytes: {bytes}"));
        }
        if let Some(bytes) = out_size {
            lines.push(format!("  - output_bytes: {bytes}"));
        }
        if let (Some(in_b), Some(out_b)) = (in_size, out_size) {
            if in_b > 0 {
                let delta = out_b as i64 - in_b as i64;
                let pct = (delta as f64 / in_b as f64) * 100.0;
                lines.push(format!("  - delta_bytes: {delta} ({pct:.2}%)"));
            }
        }
        if let Some(err) = item.error.as_deref() {
            lines.push(format!("  - error: {err}"));
        }
    }
    lines.push(String::new());

    format!("{}\n", lines.join("\n"))
}
