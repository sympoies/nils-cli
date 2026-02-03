use std::path::{Path, PathBuf};

use crate::cli_util;

pub struct ReportMetadata {
    pub project_root: PathBuf,
    pub out_path: PathBuf,
    pub report_date: String,
    pub generated_at: String,
}

pub struct ReportMetadataConfig<'a> {
    pub case_name: &'a str,
    pub out_path: Option<&'a str>,
    pub project_root: Option<&'a str>,
    pub report_dir_env: &'a str,
    pub invocation_dir: &'a Path,
}

pub fn build_report_metadata(cfg: ReportMetadataConfig<'_>) -> ReportMetadata {
    let project_root = if let Some(p) = cfg.project_root.and_then(cli_util::trim_non_empty) {
        PathBuf::from(p)
    } else {
        cli_util::find_git_root(cfg.invocation_dir)
            .unwrap_or_else(|| cfg.invocation_dir.to_path_buf())
    };

    let out_path = match cfg.out_path.and_then(cli_util::trim_non_empty) {
        Some(p) => PathBuf::from(p),
        None => {
            let stamp =
                cli_util::report_stamp_now().unwrap_or_else(|_| "00000000-0000".to_string());
            let case_slug = cli_util::slugify(cfg.case_name.trim());
            let case_slug = if case_slug.is_empty() {
                "case".to_string()
            } else {
                case_slug
            };

            let report_dir = std::env::var(cfg.report_dir_env)
                .ok()
                .and_then(|s| cli_util::trim_non_empty(&s));
            let report_dir = match report_dir {
                None => project_root.join("docs"),
                Some(d) => {
                    let p = PathBuf::from(d);
                    if p.is_absolute() {
                        p
                    } else {
                        project_root.join(p)
                    }
                }
            };

            report_dir.join(format!("{stamp}-{case_slug}-api-test-report.md"))
        }
    };

    let report_date = cli_util::report_date_now().unwrap_or_else(|_| "0000-00-00".to_string());
    let generated_at = cli_util::history_timestamp_now().unwrap_or_else(|_| "".to_string());

    ReportMetadata {
        project_root,
        out_path,
        report_date,
        generated_at,
    }
}

pub fn endpoint_note(url: Option<&str>, env: Option<&str>, implicit_note: &str) -> String {
    if url.and_then(cli_util::trim_non_empty).is_some() {
        format!("Endpoint: --url {}", url.unwrap_or_default())
    } else if env.and_then(cli_util::trim_non_empty).is_some() {
        format!("Endpoint: --env {}", env.unwrap_or_default())
    } else {
        implicit_note.to_string()
    }
}
