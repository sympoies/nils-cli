use std::path::PathBuf;

use crate::cli::{OutputFormat, ProfileInitArgs, ProfileValidateArgs};
use crate::error::CliError;
use crate::model::{ProfileInitResult, ProfileValidateResult, SuccessEnvelope};
use crate::test_mode;

pub fn run_validate(format: OutputFormat, args: &ProfileValidateArgs) -> Result<(), CliError> {
    let raw = std::fs::read_to_string(&args.file).map_err(|err| {
        CliError::runtime(format!(
            "failed to read profile file `{}`: {err}",
            args.file.display()
        ))
        .with_operation("profile.validate")
    })?;
    let value: serde_json::Value = serde_json::from_str(&raw).map_err(|err| {
        CliError::usage(format!(
            "profile file `{}` is not valid json: {err}",
            args.file.display()
        ))
        .with_operation("profile.validate")
    })?;

    let issues = collect_profile_issues(&value);
    if !issues.is_empty() {
        return Err(CliError::usage(format!(
            "profile validation failed for `{}`: {}",
            args.file.display(),
            issues.join("; ")
        ))
        .with_operation("profile.validate")
        .with_hint(
            "Fix the listed key paths or regenerate a scaffold with `macos-agent profile init`.",
        ));
    }

    let result = ProfileValidateResult {
        file: args.file.display().to_string(),
        valid: true,
        issues: Vec::new(),
    };

    match format {
        OutputFormat::Json => {
            let payload = SuccessEnvelope::new("profile.validate", result);
            println!(
                "{}",
                serde_json::to_string(&payload).map_err(|err| CliError::runtime(format!(
                    "failed to serialize json output: {err}"
                )))?
            );
        }
        OutputFormat::Text => {
            println!("profile.validate\tfile={}\tvalid=true", result.file);
        }
        OutputFormat::Tsv => {
            return Err(CliError::usage(
                "--format tsv is only supported for `windows list` and `apps list`",
            ));
        }
    }

    Ok(())
}

pub fn run_init(format: OutputFormat, args: &ProfileInitArgs) -> Result<(), CliError> {
    let output_path = resolve_profile_init_path(args.path.clone());
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            CliError::runtime(format!(
                "failed to create profile output directory `{}`: {err}",
                parent.display()
            ))
            .with_operation("profile.init")
        })?;
    }

    let scaffold = serde_json::json!({
        "profile_name": args.name,
        "arc": {
            "youtube_home_url": "https://www.youtube.com/",
            "video_tiles": [
                {"x": 460, "y": 360},
                {"x": 960, "y": 360},
                {"x": 1460, "y": 360}
            ],
            "player_focus": {"x": 960, "y": 420},
            "comment_checkpoint": {"x": 960, "y": 960}
        },
        "spotify": {
            "search_box": {"x": 220, "y": 112},
            "track_rows": [
                {"x": 760, "y": 330}
            ],
            "play_toggle": {"x": 960, "y": 1330}
        },
        "finder": {
            "window_focus": {"x": 640, "y": 220}
        }
    });

    let body = serde_json::to_vec_pretty(&scaffold).map_err(|err| {
        CliError::runtime(format!("failed to serialize profile scaffold: {err}"))
            .with_operation("profile.init")
    })?;
    std::fs::write(&output_path, body).map_err(|err| {
        CliError::runtime(format!(
            "failed to write profile scaffold `{}`: {err}",
            output_path.display()
        ))
        .with_operation("profile.init")
    })?;

    let result = ProfileInitResult {
        path: output_path.display().to_string(),
        profile_name: args.name.clone(),
    };

    match format {
        OutputFormat::Json => {
            let payload = SuccessEnvelope::new("profile.init", result);
            println!(
                "{}",
                serde_json::to_string(&payload).map_err(|err| CliError::runtime(format!(
                    "failed to serialize json output: {err}"
                )))?
            );
        }
        OutputFormat::Text => {
            println!("{}", output_path.display());
        }
        OutputFormat::Tsv => {
            return Err(CliError::usage(
                "--format tsv is only supported for `windows list` and `apps list`",
            ));
        }
    }

    Ok(())
}

fn resolve_profile_init_path(path: Option<PathBuf>) -> PathBuf {
    path.unwrap_or_else(|| {
        codex_out_dir().join(format!(
            "macos-agent-profile-{}.json",
            test_mode::timestamp_token()
        ))
    })
}

fn codex_out_dir() -> PathBuf {
    if let Ok(codex_home) = std::env::var("CODEX_HOME") {
        return PathBuf::from(codex_home).join("out");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".codex").join("out");
    }
    PathBuf::from(".codex").join("out")
}

fn collect_profile_issues(value: &serde_json::Value) -> Vec<String> {
    let mut issues = Vec::new();

    if value["profile_name"].as_str().map(|v| !v.trim().is_empty()) != Some(true) {
        issues.push("profile_name must be a non-empty string".to_string());
    }
    if value["arc"]["youtube_home_url"]
        .as_str()
        .map(|v| !v.trim().is_empty())
        != Some(true)
    {
        issues.push("arc.youtube_home_url must be a non-empty string".to_string());
    }

    validate_points(
        value,
        "arc.video_tiles",
        true,
        3,
        &mut issues,
        validate_point_bounds,
    );
    validate_point(value, "arc.player_focus", &mut issues);
    validate_point(value, "arc.comment_checkpoint", &mut issues);

    validate_point(value, "spotify.search_box", &mut issues);
    validate_points(
        value,
        "spotify.track_rows",
        true,
        1,
        &mut issues,
        validate_point_bounds,
    );
    validate_point(value, "spotify.play_toggle", &mut issues);

    validate_point(value, "finder.window_focus", &mut issues);

    issues
}

fn validate_points(
    root: &serde_json::Value,
    path: &str,
    required: bool,
    min_len: usize,
    issues: &mut Vec<String>,
    check: fn(&serde_json::Value, &str, &mut Vec<String>),
) {
    let Some(node) = value_at_path(root, path) else {
        if required {
            issues.push(format!("{path} is required"));
        }
        return;
    };

    let Some(rows) = node.as_array() else {
        issues.push(format!("{path} must be an array"));
        return;
    };

    if rows.len() < min_len {
        issues.push(format!("{path} must contain at least {min_len} points"));
    }

    for (idx, point) in rows.iter().enumerate() {
        check(point, &format!("{path}[{idx}]"), issues);
    }
}

fn validate_point(root: &serde_json::Value, path: &str, issues: &mut Vec<String>) {
    let Some(node) = value_at_path(root, path) else {
        issues.push(format!("{path} is required"));
        return;
    };
    validate_point_bounds(node, path, issues);
}

fn validate_point_bounds(node: &serde_json::Value, path: &str, issues: &mut Vec<String>) {
    let Some(x) = node["x"].as_i64() else {
        issues.push(format!("{path}.x must be an integer"));
        return;
    };
    let Some(y) = node["y"].as_i64() else {
        issues.push(format!("{path}.y must be an integer"));
        return;
    };

    if !(0..=10_000).contains(&x) {
        issues.push(format!("{path}.x must be between 0 and 10000"));
    }
    if !(0..=10_000).contains(&y) {
        issues.push(format!("{path}.y must be between 0 and 10000"));
    }
}

fn value_at_path<'a>(root: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut cursor = root;
    for segment in path.split('.') {
        cursor = cursor.get(segment)?;
    }
    Some(cursor)
}

#[cfg(test)]
mod tests {
    use super::collect_profile_issues;

    #[test]
    fn profile_issues_include_key_paths() {
        let bad = serde_json::json!({
            "profile_name": "",
            "arc": {
                "youtube_home_url": "",
                "video_tiles": [],
                "player_focus": {"x": -1, "y": 10},
                "comment_checkpoint": {"x": 10}
            },
            "spotify": {
                "search_box": {"x": 10, "y": 10},
                "track_rows": [],
                "play_toggle": {"x": 10, "y": 10}
            },
            "finder": {}
        });

        let issues = collect_profile_issues(&bad);
        assert!(issues.iter().any(|issue| issue.contains("profile_name")));
        assert!(issues.iter().any(|issue| issue.contains("arc.video_tiles")));
        assert!(issues
            .iter()
            .any(|issue| issue.contains("finder.window_focus")));
    }
}
