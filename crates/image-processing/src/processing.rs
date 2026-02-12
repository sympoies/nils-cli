use crate::cli::Operation;
use crate::model::{
    Collision, ImageInfo, ItemResult, OutputMode, SCHEMA_VERSION, SUPPORTED_CONVERT_TARGETS,
    SourceContext, Summary, SummaryOptions,
};
use crate::report::render_report_md;
use crate::svg_validate;
use crate::toolchain::{Toolchain, probe_image};
use crate::util;
use nils_common::process as common_process;
use nils_term::progress::Progress;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub fn expand_inputs(
    inputs: &[String],
    recursive: bool,
    globs: &[String],
) -> Result<Vec<PathBuf>, util::UsageError> {
    if inputs.is_empty() {
        return Err(util::UsageError {
            message: "missing --in".to_string(),
        });
    }

    let patterns: Vec<String> = globs
        .iter()
        .map(|g| g.trim().to_string())
        .filter(|g| !g.is_empty())
        .collect();

    let compiled: Vec<globset::GlobMatcher> = patterns
        .iter()
        .filter_map(|p| globset::Glob::new(p).ok())
        .map(|g| g.compile_matcher())
        .collect();

    let matches = |path: &Path| -> bool {
        if compiled.is_empty() {
            return true;
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            return false;
        };
        compiled.iter().any(|m| m.is_match(name))
    };

    let mut out: Vec<PathBuf> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();

    for raw in inputs {
        let expanded = util::expand_user(raw);
        if !expanded.exists() {
            return Err(util::UsageError {
                message: format!("input not found: {raw}"),
            });
        }
        if expanded.is_file() {
            let rp = std::fs::canonicalize(&expanded).map_err(|e| util::UsageError {
                message: format!("failed to resolve input: {raw}: {e}"),
            })?;
            if matches(&rp) && seen.insert(rp.clone()) {
                out.push(rp);
            }
            continue;
        }
        if !expanded.is_dir() {
            continue;
        }

        let mut candidates: Vec<PathBuf> = Vec::new();
        if recursive {
            for entry in walkdir::WalkDir::new(&expanded)
                .follow_links(false)
                .into_iter()
                .filter_map(Result::ok)
            {
                if entry.file_type().is_file() {
                    candidates.push(entry.path().to_path_buf());
                }
            }
        } else {
            let mut names: Vec<PathBuf> = std::fs::read_dir(&expanded)
                .map_err(|e| util::UsageError {
                    message: format!("failed to read dir: {}: {e}", expanded.display()),
                })?
                .filter_map(Result::ok)
                .map(|e| e.path())
                .collect();
            names.sort_by_key(|p| p.to_string_lossy().to_string());
            candidates.extend(names);
        }

        candidates.sort_by_key(|p| p.to_string_lossy().to_string());
        for c in candidates {
            if !c.is_file() {
                continue;
            }
            if !matches(&c) {
                continue;
            }
            let rp = match std::fs::canonicalize(&c) {
                Ok(p) => p,
                Err(_) => continue,
            };
            if seen.insert(rp.clone()) {
                out.push(rp);
            }
        }
    }

    if out.is_empty() {
        return Err(util::UsageError {
            message: "no input files resolved from --in/--glob".to_string(),
        });
    }
    Ok(out)
}

pub fn validate_output_mode(
    subcommand: Operation,
    out: Option<&str>,
    out_dir: Option<&str>,
    in_place: bool,
    yes: bool,
) -> Result<Option<OutputMode>, util::UsageError> {
    if subcommand == Operation::Info {
        if out.is_some() || out_dir.is_some() || in_place {
            return Err(util::UsageError {
                message: "info does not write outputs; do not pass --out/--out-dir/--in-place"
                    .to_string(),
            });
        }
        return Ok(None);
    }

    let chosen = [out.is_some(), out_dir.is_some(), in_place];
    if chosen.iter().filter(|x| **x).count() != 1 {
        return Err(util::UsageError {
            message: "must specify exactly one output mode: --out, --out-dir, or --in-place"
                .to_string(),
        });
    }
    if in_place && !yes {
        return Err(util::UsageError {
            message: "--in-place is destructive and requires --yes".to_string(),
        });
    }

    if let Some(out) = out {
        return Ok(Some(OutputMode {
            mode: "out",
            out: Some(util::expand_user(out)),
            out_dir: None,
        }));
    }
    if let Some(out_dir) = out_dir {
        return Ok(Some(OutputMode {
            mode: "out_dir",
            out: None,
            out_dir: Some(util::expand_user(out_dir)),
        }));
    }
    Ok(Some(OutputMode {
        mode: "in_place",
        out: None,
        out_dir: None,
    }))
}

pub struct ProcessArgs<'a> {
    pub toolchain: Option<&'a Toolchain>,
    pub backend: &'a str,
    pub repo_root: &'a Path,
    pub run_dir: Option<&'a Path>,
    pub progress: Progress,
    pub subcommand: Operation,
    pub inputs: &'a [PathBuf],
    pub from_svg_input: Option<&'a Path>,
    pub output_mode: Option<&'a OutputMode>,
    pub overwrite: bool,
    pub dry_run: bool,
    pub auto_orient_enabled: bool,
    pub strip_metadata: bool,
    pub background: Option<&'a str>,
    pub report_enabled: bool,
    pub json_enabled: bool,
    pub convert_to: Option<&'a str>,
    pub quality: Option<i32>,
    pub resize_scale: Option<f64>,
    pub resize_width: Option<i32>,
    pub resize_height: Option<i32>,
    pub resize_aspect: Option<(i32, i32)>,
    pub resize_fit: Option<&'a str>,
    pub no_pre_upscale: bool,
    pub rotate_degrees: Option<i32>,
    pub crop_rect: Option<(i32, i32, i32, i32)>,
    pub crop_size: Option<(i32, i32)>,
    pub crop_aspect: Option<(i32, i32)>,
    pub crop_gravity: &'a str,
    pub pad_width: Option<i32>,
    pub pad_height: Option<i32>,
    pub pad_gravity: &'a str,
    pub optimize_lossless: bool,
    pub optimize_progressive: bool,
}

pub fn process_items(args: ProcessArgs<'_>) -> anyhow::Result<Summary> {
    let ProcessArgs {
        toolchain,
        backend,
        repo_root,
        run_dir,
        progress,
        subcommand,
        inputs,
        from_svg_input,
        output_mode,
        overwrite,
        dry_run,
        auto_orient_enabled,
        strip_metadata,
        background,
        report_enabled,
        json_enabled,
        convert_to,
        quality,
        resize_scale,
        resize_width,
        resize_height,
        resize_aspect,
        resize_fit,
        no_pre_upscale,
        rotate_degrees,
        crop_rect,
        crop_size,
        crop_aspect,
        crop_gravity,
        pad_width,
        pad_height,
        pad_gravity,
        optimize_lossless,
        optimize_progressive,
    } = args;

    let from_svg_mode = from_svg_input.is_some();
    let source = SourceContext {
        mode: if from_svg_mode {
            "from_svg".to_string()
        } else if subcommand == Operation::SvgValidate {
            "svg_validate".to_string()
        } else {
            "inputs".to_string()
        },
        from_svg: from_svg_input.map(|p| util::maybe_relpath(p, repo_root)),
    };

    let toolchain = if from_svg_mode || subcommand == Operation::SvgValidate {
        None
    } else {
        Some(toolchain.ok_or_else(|| {
            anyhow::anyhow!("internal error: ImageMagick toolchain missing for this operation")
        })?)
    };

    let _ = json_enabled;

    if report_enabled && subcommand == Operation::Info {
        return Err(util::usage_err("--report is not supported for info"));
    }

    if !from_svg_mode
        && !matches!(subcommand, Operation::Info | Operation::SvgValidate)
        && let Some(mode) = output_mode
        && mode.mode == "out"
        && inputs.len() != 1
    {
        return Err(util::usage_err("--out requires exactly one input file"));
    }

    let from_svg_doc = if let Some(path) = from_svg_input {
        Some(svg_validate::sanitize_svg_file(path)?)
    } else {
        None
    };

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    #[derive(Clone)]
    struct PlannedItem {
        input_path: PathBuf,
        output_path: Option<PathBuf>,
    }

    let mut planned: Vec<PlannedItem> = Vec::new();
    let mut collisions: Vec<Collision> = Vec::new();
    let mut out_by_path: HashMap<PathBuf, PathBuf> = HashMap::new();

    let derive_out_path = |inp: &PathBuf| -> anyhow::Result<PathBuf> {
        let Some(mode) = output_mode else {
            return Err(util::usage_err("internal error: output_mode missing"));
        };
        if mode.mode == "in_place" {
            return Ok(inp.clone());
        }
        if mode.mode == "out" {
            return Ok(mode.out.clone().expect("out"));
        }
        let out_dir = mode.out_dir.clone().expect("out_dir");
        let in_ext = ext_normalize(inp);
        let mut out_ext = in_ext.clone();

        if subcommand == Operation::Convert {
            let Some(to) = convert_to else {
                return Err(util::usage_err("internal error: convert_to missing"));
            };
            out_ext = to.to_string();
        } else if subcommand == Operation::Optimize {
            out_ext = in_ext;
        }

        let stem = inp.file_stem().unwrap_or_else(|| inp.as_os_str());
        let filename = if out_ext.is_empty() {
            inp.file_name()
                .unwrap_or(stem)
                .to_string_lossy()
                .to_string()
        } else {
            format!("{}.{}", stem.to_string_lossy(), out_ext)
        };
        Ok(out_dir.join(filename))
    };

    if subcommand != Operation::Info {
        let mode = output_mode.expect("output_mode required");

        if from_svg_mode {
            let from_svg_path = from_svg_input.expect("from_svg_input").to_path_buf();
            let out_path = mode
                .out
                .clone()
                .ok_or_else(|| util::usage_err("convert --from-svg requires --out"))?;
            let out_abs = util::abs_path(&out_path, &cwd);
            let target = svg_validate::parse_from_svg_target(convert_to)?;
            let ext = ext_normalize(&out_abs);
            if ext != target {
                return Err(util::usage_err(format!(
                    "--out extension must match --to {target}: {}",
                    out_abs.display()
                )));
            }
            planned.push(PlannedItem {
                input_path: from_svg_path,
                output_path: Some(out_abs),
            });
        } else {
            for inp in inputs {
                let out_path = derive_out_path(inp)?;
                let out_abs = util::abs_path(&out_path, &cwd);

                if subcommand == Operation::Convert
                    && let Some(to) = convert_to
                {
                    let ext = ext_normalize(&out_abs);
                    if ext != to {
                        return Err(util::usage_err(format!(
                            "--out extension must match --to {to}: {}",
                            out_abs.display()
                        )));
                    }
                }

                if subcommand == Operation::SvgValidate {
                    let ext = ext_normalize(&out_abs);
                    if ext != "svg" {
                        return Err(util::usage_err("svg-validate --out must end with .svg"));
                    }
                }

                if subcommand == Operation::Optimize {
                    let in_ext = ext_normalize(inp);
                    let out_ext = ext_normalize(&out_abs);
                    if out_ext != in_ext {
                        return Err(util::usage_err(
                            "optimize does not change formats; output extension must match input",
                        ));
                    }
                } else if !matches!(subcommand, Operation::Convert | Operation::SvgValidate) {
                    let in_ext = ext_normalize(inp);
                    let out_ext = ext_normalize(&out_abs);
                    if out_ext != in_ext {
                        return Err(util::usage_err(
                            "only convert changes formats; output extension must match input",
                        ));
                    }
                }

                if mode.mode != "in_place" {
                    if let Some(prev) = out_by_path.get(&out_abs) {
                        let filename = out_abs
                            .file_name()
                            .map(|x| x.to_string_lossy().to_string())
                            .unwrap_or_else(|| out_abs.to_string_lossy().to_string());
                        collisions.push(Collision {
                            path: out_abs.to_string_lossy().to_string(),
                            reason: format!("multiple inputs map to the same output ({filename})"),
                        });
                        let _ = prev;
                    }
                    out_by_path.insert(out_abs.clone(), inp.clone());
                }

                planned.push(PlannedItem {
                    input_path: inp.clone(),
                    output_path: Some(out_abs),
                });
            }
        }

        if !collisions.is_empty() {
            return Err(util::usage_err(
                "output collisions detected; adjust --out-dir or inputs",
            ));
        }

        if mode.mode != "in_place" {
            for item in &planned {
                let out_abs = item.output_path.as_ref().expect("out_abs");
                util::check_overwrite(out_abs, overwrite)?;
            }

            if report_enabled && let Some(run_dir) = run_dir {
                let report_path = run_dir.join("report.md");
                util::check_overwrite(&report_path, overwrite)?;
            }
        }
    } else {
        for p in inputs {
            planned.push(PlannedItem {
                input_path: p.clone(),
                output_path: None,
            });
        }
    }

    let mut commands: Vec<String> = Vec::new();
    let warnings: Vec<String> = Vec::new();
    let skipped: Vec<serde_json::Value> = Vec::new();
    let mut items: Vec<ItemResult> = Vec::new();

    if subcommand != Operation::Info {
        let mode = output_mode.expect("output_mode required");
        if !dry_run && mode.mode == "out_dir" {
            let out_dir = mode.out_dir.clone().expect("out_dir");
            let out_dir_abs = util::abs_path(&out_dir, &cwd);
            std::fs::create_dir_all(out_dir_abs)?;
        }
        if !dry_run && mode.mode == "out" {
            let out = mode.out.clone().expect("out");
            let out_abs = util::abs_path(&out, &cwd);
            util::ensure_parent_dir(&out_abs, dry_run)?;
        }
    }

    for planned_item in planned {
        let PlannedItem {
            input_path: inp,
            output_path: out_abs,
        } = planned_item;
        progress.set_message(util::maybe_relpath(&inp, repo_root));

        let mut input_info = if from_svg_mode {
            let doc = from_svg_doc.as_ref().expect("from_svg_doc");
            ImageInfo {
                format: Some("SVG".to_string()),
                width: Some(doc.width as i32),
                height: Some(doc.height as i32),
                channels: None,
                alpha: Some(doc.uses_alpha),
                exif_orientation: None,
                size_bytes: std::fs::metadata(&inp).ok().map(|m| m.len()),
            }
        } else if subcommand == Operation::SvgValidate {
            ImageInfo {
                format: Some("SVG".to_string()),
                size_bytes: std::fs::metadata(&inp).ok().map(|m| m.len()),
                ..Default::default()
            }
        } else {
            toolchain
                .map(|tc| probe_image(tc, &inp))
                .unwrap_or_default()
        };
        let input_alpha = input_info.alpha.unwrap_or(false);

        let in_ext = ext_normalize(&inp);
        let out_ext = out_abs
            .as_ref()
            .map(|p| ext_normalize(p))
            .unwrap_or_default();

        let mut item_cmds: Vec<String> = Vec::new();
        let item_warnings: Vec<String> = Vec::new();
        let mut item_error: Option<String> = None;
        let mut output_info: Option<ImageInfo> = None;

        let result: anyhow::Result<()> = (|| {
            if from_svg_mode {
                let output_path = out_abs
                    .as_deref()
                    .ok_or_else(|| util::usage_err("internal error: missing output path"))?;
                let target = svg_validate::parse_from_svg_target(convert_to)?;
                let mut cmd = vec![
                    "image-processing".to_string(),
                    "convert".to_string(),
                    "--from-svg".to_string(),
                    inp.to_string_lossy().to_string(),
                    "--to".to_string(),
                    target.to_string(),
                    "--out".to_string(),
                    output_path.to_string_lossy().to_string(),
                ];
                if dry_run {
                    cmd.push("--dry-run".to_string());
                }
                item_cmds.push(util::command_str(&cmd));

                let doc = from_svg_doc.as_ref().expect("from_svg_doc");
                let info = svg_validate::render_svg_to_output(doc, target, output_path, dry_run)?;
                if !dry_run {
                    output_info = Some(info);
                }
                return Ok(());
            }

            if subcommand == Operation::SvgValidate {
                let output_path = out_abs
                    .as_deref()
                    .ok_or_else(|| util::usage_err("internal error: missing output path"))?;
                let mut cmd = vec![
                    "image-processing".to_string(),
                    "svg-validate".to_string(),
                    "--in".to_string(),
                    inp.to_string_lossy().to_string(),
                    "--out".to_string(),
                    output_path.to_string_lossy().to_string(),
                ];
                if dry_run {
                    cmd.push("--dry-run".to_string());
                }
                item_cmds.push(util::command_str(&cmd));

                let validation =
                    svg_validate::run_svg_validate_command(&inp, output_path, dry_run)?;
                if !validation.sanitized {
                    return Err(svg_validate::diagnostics_to_error(
                        &inp,
                        &validation.diagnostics,
                    ));
                }
                input_info.width = validation.width.map(|w| w as i32);
                input_info.height = validation.height.map(|h| h as i32);
                if !dry_run {
                    output_info = Some(ImageInfo {
                        format: Some("SVG".to_string()),
                        width: validation.width.map(|w| w as i32),
                        height: validation.height.map(|h| h as i32),
                        channels: None,
                        alpha: Some(false),
                        exif_orientation: None,
                        size_bytes: std::fs::metadata(output_path).ok().map(|m| m.len()),
                    });
                }
                return Ok(());
            }

            let toolchain = toolchain.expect("legacy path must have ImageMagick toolchain");

            #[allow(unreachable_patterns)]
            match subcommand {
                Operation::Info => Ok(()),
                Operation::AutoOrient => {
                    let out_abs = out_abs.as_ref().expect("out_abs");
                    let tmp = util::safe_write_path(out_abs, dry_run);
                    let mut cmd = build_magick_cmd(toolchain, &inp)?;
                    cmd.push("-auto-orient".to_string());
                    if strip_metadata {
                        cmd.push("-strip".to_string());
                    }
                    cmd.push(tmp.to_string_lossy().to_string());
                    item_cmds.push(util::command_str(&cmd));
                    let (rc, _stdout, stderr) = run_one_magick(&cmd, dry_run)?;
                    if rc != 0 {
                        return Err(anyhow::anyhow!(
                            "{}",
                            stderr.trim().to_string().if_empty("auto-orient failed")
                        ));
                    }
                    if !dry_run {
                        util::atomic_replace(&tmp, out_abs, dry_run)?;
                        output_info = Some(probe_image(toolchain, out_abs));
                    }
                    Ok(())
                }
                Operation::Convert => {
                    let out_abs = out_abs.as_ref().expect("out_abs");
                    let Some(convert_to) = convert_to else {
                        return Err(util::usage_err("internal error: convert_to missing"));
                    };
                    if !SUPPORTED_CONVERT_TARGETS.contains(&convert_to) {
                        return Err(anyhow::anyhow!(
                            "unsupported --to: {convert_to} (supported: png|jpg|webp)"
                        ));
                    }

                    let tmp = util::safe_write_path(out_abs, dry_run);
                    let mut cmd = build_magick_cmd(toolchain, &inp)?;
                    if auto_orient_enabled {
                        cmd.push("-auto-orient".to_string());
                    }

                    if convert_to == "jpg" {
                        if input_alpha && background.is_none() {
                            return Err(anyhow::anyhow!(
                                "{}",
                                require_background(
                                    "alpha input cannot be converted to JPEG without a background"
                                )?
                            ));
                        }
                        if let Some(bg) = background {
                            cmd.extend([
                                "-background".to_string(),
                                bg.to_string(),
                                "-alpha".to_string(),
                                "remove".to_string(),
                                "-alpha".to_string(),
                                "off".to_string(),
                            ]);
                        }
                    }

                    if let Some(q) = quality {
                        if !(0..=100).contains(&q) {
                            return Err(anyhow::anyhow!("--quality must be 0..100"));
                        }
                        cmd.extend(["-quality".to_string(), q.to_string()]);
                    }

                    if strip_metadata {
                        cmd.push("-strip".to_string());
                    }

                    cmd.push(tmp.to_string_lossy().to_string());
                    item_cmds.push(util::command_str(&cmd));

                    let (rc, _stdout, stderr) = run_one_magick(&cmd, dry_run)?;
                    if rc != 0 {
                        return Err(anyhow::anyhow!(
                            "{}",
                            stderr.trim().to_string().if_empty("convert failed")
                        ));
                    }
                    if !dry_run {
                        util::atomic_replace(&tmp, out_abs, dry_run)?;
                        output_info = Some(probe_image(toolchain, out_abs));
                    }
                    Ok(())
                }
                Operation::Resize => {
                    let out_abs = out_abs.as_ref().expect("out_abs");
                    let (orig_w, orig_h) = match (input_info.width, input_info.height) {
                        (Some(w), Some(h)) => (w, h),
                        _ => {
                            return Err(anyhow::anyhow!(
                                "unable to read input dimensions for resize"
                            ));
                        }
                    };
                    let (tw, th, fit_mode, uses_box) = compute_resize_box(
                        orig_w,
                        orig_h,
                        resize_scale,
                        resize_width,
                        resize_height,
                        resize_aspect,
                        resize_fit,
                    )?;

                    let tmp = util::safe_write_path(out_abs, dry_run);
                    let mut cmd = build_magick_cmd(toolchain, &inp)?;
                    if auto_orient_enabled {
                        cmd.push("-auto-orient".to_string());
                    }

                    let pre_upscale = !no_pre_upscale;
                    if pre_upscale {
                        cmd.extend(["-resize".to_string(), "200%".to_string()]);
                    }

                    if uses_box {
                        let fit_mode = fit_mode.expect("fit_mode");
                        let box_s = format!("{tw}x{th}");
                        let gravity = "center";
                        match fit_mode.as_str() {
                            "stretch" => cmd.extend(["-resize".to_string(), format!("{box_s}!")]),
                            "cover" => cmd.extend([
                                "-resize".to_string(),
                                format!("{box_s}^"),
                                "-gravity".to_string(),
                                gravity.to_string(),
                                "-extent".to_string(),
                                box_s.clone(),
                            ]),
                            "contain" => {
                                cmd.extend(["-resize".to_string(), box_s.clone()]);
                                let mut bg = background.map(|s| s.to_string());
                                if bg.is_none() && output_supports_alpha(&out_ext) {
                                    bg = Some("none".to_string());
                                }
                                if bg.is_none() && is_non_alpha_format(&out_ext) {
                                    return Err(anyhow::anyhow!(
                                        "{}",
                                        require_background(
                                            "contain fit requires padding background for non-alpha outputs"
                                        )?
                                    ));
                                }
                                if let Some(bg) = bg {
                                    cmd.extend(["-background".to_string(), bg]);
                                }
                                cmd.extend([
                                    "-gravity".to_string(),
                                    gravity.to_string(),
                                    "-extent".to_string(),
                                    box_s.clone(),
                                ]);
                            }
                            _ => {
                                return Err(anyhow::anyhow!(
                                    "internal error: unknown fit_mode {fit_mode}"
                                ));
                            }
                        }
                    } else {
                        cmd.extend(["-resize".to_string(), format!("{tw}x{th}!")]);
                    }

                    if strip_metadata {
                        cmd.push("-strip".to_string());
                    }
                    cmd.push(tmp.to_string_lossy().to_string());
                    item_cmds.push(util::command_str(&cmd));
                    let (rc, _stdout, stderr) = run_one_magick(&cmd, dry_run)?;
                    if rc != 0 {
                        return Err(anyhow::anyhow!(
                            "{}",
                            stderr.trim().to_string().if_empty("resize failed")
                        ));
                    }
                    if !dry_run {
                        util::atomic_replace(&tmp, out_abs, dry_run)?;
                        output_info = Some(probe_image(toolchain, out_abs));
                    }
                    Ok(())
                }
                Operation::Rotate => {
                    let out_abs = out_abs.as_ref().expect("out_abs");
                    let degrees = rotate_degrees
                        .ok_or_else(|| anyhow::anyhow!("rotate requires --degrees"))?;
                    let tmp = util::safe_write_path(out_abs, dry_run);
                    let mut cmd = build_magick_cmd(toolchain, &inp)?;
                    if auto_orient_enabled {
                        cmd.push("-auto-orient".to_string());
                    }

                    let mut bg = background.map(|s| s.to_string());
                    if degrees % 90 != 0 {
                        if bg.is_none() && output_supports_alpha(&out_ext) {
                            bg = Some("none".to_string());
                        }
                        if bg.is_none() && is_non_alpha_format(&out_ext) {
                            return Err(anyhow::anyhow!(
                                "{}",
                                require_background(
                                    "non-right-angle rotation requires a background for JPEG outputs"
                                )?
                            ));
                        }
                        if let Some(bg) = bg {
                            cmd.extend(["-background".to_string(), bg]);
                        }
                    }

                    cmd.extend(["-rotate".to_string(), degrees.to_string()]);
                    if strip_metadata {
                        cmd.push("-strip".to_string());
                    }
                    cmd.push(tmp.to_string_lossy().to_string());
                    item_cmds.push(util::command_str(&cmd));
                    let (rc, _stdout, stderr) = run_one_magick(&cmd, dry_run)?;
                    if rc != 0 {
                        return Err(anyhow::anyhow!(
                            "{}",
                            stderr.trim().to_string().if_empty("rotate failed")
                        ));
                    }
                    if !dry_run {
                        util::atomic_replace(&tmp, out_abs, dry_run)?;
                        output_info = Some(probe_image(toolchain, out_abs));
                    }
                    Ok(())
                }
                Operation::Crop => {
                    let out_abs = out_abs.as_ref().expect("out_abs");
                    let (orig_w, orig_h) = match (input_info.width, input_info.height) {
                        (Some(w), Some(h)) => (w, h),
                        _ => {
                            return Err(anyhow::anyhow!(
                                "unable to read input dimensions for crop"
                            ));
                        }
                    };

                    if [
                        crop_rect.is_some(),
                        crop_size.is_some(),
                        crop_aspect.is_some(),
                    ]
                    .into_iter()
                    .filter(|x| *x)
                    .count()
                        != 1
                    {
                        return Err(anyhow::anyhow!(
                            "crop requires exactly one of: --rect, --size, --aspect"
                        ));
                    }

                    let (cw, ch, cx, cy) = if let Some((cw, ch, cx, cy)) = crop_rect {
                        (cw, ch, cx, cy)
                    } else if let Some((cw, ch)) = crop_size {
                        (cw, ch, 0, 0)
                    } else {
                        let (aw, ah) = crop_aspect.expect("crop_aspect");
                        let target_aspect = aw as f64 / ah as f64;
                        let orig_aspect = orig_w as f64 / orig_h as f64;
                        if orig_aspect > target_aspect {
                            let ch = orig_h;
                            let cw = ((ch as f64) * target_aspect).round() as i32;
                            (cw.max(1), ch, 0, 0)
                        } else {
                            let cw = orig_w;
                            let ch = ((cw as f64) / target_aspect).round() as i32;
                            (cw, ch.max(1), 0, 0)
                        }
                    };

                    if cw <= 0 || ch <= 0 {
                        return Err(anyhow::anyhow!("invalid crop dimensions"));
                    }
                    if cw > orig_w || ch > orig_h {
                        return Err(anyhow::anyhow!("crop size exceeds input dimensions"));
                    }

                    let tmp = util::safe_write_path(out_abs, dry_run);
                    let mut cmd = build_magick_cmd(toolchain, &inp)?;
                    if auto_orient_enabled {
                        cmd.push("-auto-orient".to_string());
                    }
                    if crop_rect.is_some() {
                        cmd.extend([
                            "-crop".to_string(),
                            format!("{cw}x{ch}+{cx}+{cy}"),
                            "+repage".to_string(),
                        ]);
                    } else {
                        cmd.extend([
                            "-gravity".to_string(),
                            crop_gravity.to_string(),
                            "-crop".to_string(),
                            format!("{cw}x{ch}+{cx}+{cy}"),
                            "+repage".to_string(),
                        ]);
                    }
                    if strip_metadata {
                        cmd.push("-strip".to_string());
                    }
                    cmd.push(tmp.to_string_lossy().to_string());
                    item_cmds.push(util::command_str(&cmd));
                    let (rc, _stdout, stderr) = run_one_magick(&cmd, dry_run)?;
                    if rc != 0 {
                        return Err(anyhow::anyhow!(
                            "{}",
                            stderr.trim().to_string().if_empty("crop failed")
                        ));
                    }
                    if !dry_run {
                        util::atomic_replace(&tmp, out_abs, dry_run)?;
                        output_info = Some(probe_image(toolchain, out_abs));
                    }
                    Ok(())
                }
                Operation::Pad => {
                    let out_abs = out_abs.as_ref().expect("out_abs");
                    let (orig_w, orig_h) = match (input_info.width, input_info.height) {
                        (Some(w), Some(h)) => (w, h),
                        _ => {
                            return Err(anyhow::anyhow!("unable to read input dimensions for pad"));
                        }
                    };
                    let (pw, ph) = match (pad_width, pad_height) {
                        (Some(w), Some(h)) => (w, h),
                        _ => return Err(anyhow::anyhow!("pad requires --width and --height")),
                    };
                    if pw < orig_w || ph < orig_h {
                        return Err(anyhow::anyhow!(
                            "pad target must be >= input dimensions (use crop or resize)"
                        ));
                    }
                    let tmp = util::safe_write_path(out_abs, dry_run);
                    let mut cmd = build_magick_cmd(toolchain, &inp)?;
                    if auto_orient_enabled {
                        cmd.push("-auto-orient".to_string());
                    }

                    let mut bg = background.map(|s| s.to_string());
                    if bg.is_none() && output_supports_alpha(&out_ext) {
                        bg = Some("none".to_string());
                    }
                    if bg.is_none() && is_non_alpha_format(&out_ext) {
                        return Err(anyhow::anyhow!(
                            "{}",
                            require_background("pad requires a background for non-alpha outputs")?
                        ));
                    }
                    if let Some(bg) = bg {
                        cmd.extend(["-background".to_string(), bg]);
                    }

                    cmd.extend([
                        "-gravity".to_string(),
                        pad_gravity.to_string(),
                        "-extent".to_string(),
                        format!("{pw}x{ph}"),
                    ]);
                    if strip_metadata {
                        cmd.push("-strip".to_string());
                    }
                    cmd.push(tmp.to_string_lossy().to_string());
                    item_cmds.push(util::command_str(&cmd));
                    let (rc, _stdout, stderr) = run_one_magick(&cmd, dry_run)?;
                    if rc != 0 {
                        return Err(anyhow::anyhow!(
                            "{}",
                            stderr.trim().to_string().if_empty("pad failed")
                        ));
                    }
                    if !dry_run {
                        util::atomic_replace(&tmp, out_abs, dry_run)?;
                        output_info = Some(probe_image(toolchain, out_abs));
                    }
                    Ok(())
                }
                Operation::Flip | Operation::Flop => {
                    let out_abs = out_abs.as_ref().expect("out_abs");
                    let tmp = util::safe_write_path(out_abs, dry_run);
                    let mut cmd = build_magick_cmd(toolchain, &inp)?;
                    if auto_orient_enabled {
                        cmd.push("-auto-orient".to_string());
                    }
                    cmd.push(format!("-{}", subcommand.as_str()));
                    if strip_metadata {
                        cmd.push("-strip".to_string());
                    }
                    cmd.push(tmp.to_string_lossy().to_string());
                    item_cmds.push(util::command_str(&cmd));
                    let (rc, _stdout, stderr) = run_one_magick(&cmd, dry_run)?;
                    if rc != 0 {
                        return Err(anyhow::anyhow!(
                            "{}",
                            stderr
                                .trim()
                                .to_string()
                                .if_empty(&format!("{} failed", subcommand.as_str()))
                        ));
                    }
                    if !dry_run {
                        util::atomic_replace(&tmp, out_abs, dry_run)?;
                        output_info = Some(probe_image(toolchain, out_abs));
                    }
                    Ok(())
                }
                Operation::Optimize => {
                    let out_abs = out_abs.as_ref().expect("out_abs");
                    let tmp = util::safe_write_path(out_abs, dry_run);
                    if let Some(q) = quality
                        && !(0..=100).contains(&q)
                    {
                        return Err(anyhow::anyhow!("--quality must be 0..100"));
                    }

                    if out_ext == "jpg" {
                        if in_ext != "jpg" {
                            return Err(anyhow::anyhow!("optimize for jpg expects jpg input"));
                        }
                        let q = quality.unwrap_or(85);

                        if let (Some(cjpeg), Some(djpeg)) =
                            (toolchain.cjpeg.as_ref(), toolchain.djpeg.as_ref())
                        {
                            let djpeg_cmd = vec![djpeg.clone(), inp.to_string_lossy().to_string()];
                            let mut cjpeg_cmd = vec![
                                cjpeg.clone(),
                                "-quality".to_string(),
                                q.to_string(),
                                "-optimize".to_string(),
                            ];
                            if optimize_progressive {
                                cjpeg_cmd.push("-progressive".to_string());
                            }
                            cjpeg_cmd.extend([
                                "-outfile".to_string(),
                                tmp.to_string_lossy().to_string(),
                            ]);

                            item_cmds.push(format!(
                                "{} | {}",
                                util::command_str(&djpeg_cmd),
                                util::command_str(&cjpeg_cmd)
                            ));

                            if !dry_run {
                                run_djpeg_cjpeg_pipeline(&djpeg_cmd, &cjpeg_cmd)?;
                            }
                        } else {
                            let mut cmd = build_magick_cmd(toolchain, &inp)?;
                            if auto_orient_enabled {
                                cmd.push("-auto-orient".to_string());
                            }
                            cmd.extend(["-quality".to_string(), q.to_string()]);
                            if optimize_progressive {
                                cmd.extend(["-interlace".to_string(), "Plane".to_string()]);
                            }
                            if strip_metadata {
                                cmd.push("-strip".to_string());
                            }
                            cmd.push(tmp.to_string_lossy().to_string());
                            item_cmds.push(util::command_str(&cmd));
                            let (rc, _stdout, stderr) = run_one_magick(&cmd, dry_run)?;
                            if rc != 0 {
                                return Err(anyhow::anyhow!(
                                    "{}",
                                    stderr.trim().to_string().if_empty("optimize jpg failed")
                                ));
                            }
                        }
                    } else if out_ext == "webp" {
                        if in_ext != "webp" {
                            return Err(anyhow::anyhow!("optimize for webp expects webp input"));
                        }
                        let q = quality.unwrap_or(80);

                        if let (Some(cwebp), Some(dwebp)) =
                            (toolchain.cwebp.as_ref(), toolchain.dwebp.as_ref())
                        {
                            let uuid = uuid::Uuid::new_v4().simple().to_string();
                            let short = &uuid[..8];
                            let tmp_pam = tmp
                                .parent()
                                .unwrap_or_else(|| Path::new("."))
                                .join(format!(".tmp-{short}.pam"));

                            let dwebp_cmd = vec![
                                dwebp.clone(),
                                inp.to_string_lossy().to_string(),
                                "-pam".to_string(),
                                "-o".to_string(),
                                tmp_pam.to_string_lossy().to_string(),
                            ];

                            let mut cwebp_cmd: Vec<String> = vec![cwebp.clone()];
                            if optimize_lossless {
                                cwebp_cmd.push("-lossless".to_string());
                            } else {
                                cwebp_cmd.extend(["-q".to_string(), q.to_string()]);
                            }
                            if strip_metadata {
                                cwebp_cmd.extend(["-metadata".to_string(), "none".to_string()]);
                            }
                            cwebp_cmd.extend([
                                tmp_pam.to_string_lossy().to_string(),
                                "-o".to_string(),
                                tmp.to_string_lossy().to_string(),
                            ]);

                            item_cmds.push(util::command_str(&dwebp_cmd));
                            item_cmds.push(util::command_str(&cwebp_cmd));

                            if !dry_run {
                                run_capture(&dwebp_cmd, "dwebp failed")?;
                                run_capture(&cwebp_cmd, "cwebp failed")?;
                                let _ = std::fs::remove_file(&tmp_pam);
                            }
                        } else {
                            let mut cmd = build_magick_cmd(toolchain, &inp)?;
                            if auto_orient_enabled {
                                cmd.push("-auto-orient".to_string());
                            }
                            if optimize_lossless {
                                cmd.extend([
                                    "-define".to_string(),
                                    "webp:lossless=true".to_string(),
                                ]);
                            } else {
                                cmd.extend(["-quality".to_string(), q.to_string()]);
                            }
                            if strip_metadata {
                                cmd.push("-strip".to_string());
                            }
                            cmd.push(tmp.to_string_lossy().to_string());
                            item_cmds.push(util::command_str(&cmd));
                            let (rc, _stdout, stderr) = run_one_magick(&cmd, dry_run)?;
                            if rc != 0 {
                                return Err(anyhow::anyhow!(
                                    "{}",
                                    stderr.trim().to_string().if_empty("optimize webp failed")
                                ));
                            }
                        }
                    } else {
                        return Err(anyhow::anyhow!(
                            "optimize currently supports only jpg/webp outputs"
                        ));
                    }

                    if !dry_run {
                        util::atomic_replace(&tmp, out_abs, dry_run)?;
                        output_info = Some(probe_image(toolchain, out_abs));
                    }
                    Ok(())
                }
                _ => Ok(()),
            }
        })();

        if let Err(err) = result {
            item_error = Some(err.to_string());
        }

        for c in &item_cmds {
            commands.push(c.clone());
        }

        items.push(ItemResult {
            input_path: util::maybe_relpath(&inp, repo_root),
            output_path: out_abs.as_ref().map(|p| util::maybe_relpath(p, repo_root)),
            status: if item_error.is_some() {
                "error".to_string()
            } else {
                "ok".to_string()
            },
            input_info,
            output_info,
            commands: item_cmds,
            warnings: item_warnings,
            error: item_error,
        });

        progress.inc(1);
    }

    let mut report_path: Option<String> = None;
    if report_enabled && let Some(run_dir) = run_dir {
        let run_id = run_dir
            .file_name()
            .map(|x| x.to_string_lossy().to_string())
            .unwrap_or_default();
        let report_md = render_report_md(
            &run_id,
            subcommand.as_str(),
            &source,
            &items,
            &commands,
            dry_run,
        );
        let report_file = run_dir.join("report.md");
        std::fs::write(&report_file, report_md)?;
        report_path = Some(util::maybe_relpath(&report_file, repo_root));
    }

    let summary = Summary {
        schema_version: SCHEMA_VERSION,
        run_id: run_dir.and_then(|p| p.file_name().map(|x| x.to_string_lossy().to_string())),
        cwd: cwd.to_string_lossy().to_string(),
        operation: subcommand.as_str().to_string(),
        backend: backend.to_string(),
        source,
        report_path: report_path.clone(),
        dry_run,
        options: SummaryOptions {
            overwrite,
            auto_orient: if matches!(subcommand, Operation::Info | Operation::AutoOrient) {
                None
            } else {
                Some(auto_orient_enabled)
            },
            strip_metadata,
            background: background.map(|s| s.to_string()),
            report: report_enabled,
        },
        commands,
        collisions,
        skipped,
        warnings,
        items,
    };

    if let Some(run_dir) = run_dir {
        let summary_file = run_dir.join("summary.json");
        let json = serde_json::to_string_pretty(&summary)?;
        std::fs::write(&summary_file, json)?;
    }

    progress.finish_with_message("done");

    Ok(summary)
}
fn parse_aspect(value: &str) -> anyhow::Result<(i32, i32)> {
    let s = value.trim();
    let (w, h) = s
        .split_once(':')
        .ok_or_else(|| anyhow::anyhow!("invalid aspect: {value:?} (expected W:H)"))?;
    let w = w
        .trim()
        .parse::<i32>()
        .map_err(|_| anyhow::anyhow!("invalid aspect: {value:?} (expected W:H)"))?;
    let h = h
        .trim()
        .parse::<i32>()
        .map_err(|_| anyhow::anyhow!("invalid aspect: {value:?} (expected W:H)"))?;
    if w <= 0 || h <= 0 {
        return Err(anyhow::anyhow!(
            "invalid aspect: {value:?} (W and H must be > 0)"
        ));
    }
    Ok((w, h))
}

pub fn parse_aspect_opt(value: Option<&str>) -> anyhow::Result<Option<(i32, i32)>> {
    value.map(parse_aspect).transpose()
}

pub fn parse_geometry(value: &str) -> anyhow::Result<(i32, i32, i32, i32)> {
    // WxH+X+Y (matches python: requires two '+' separators; X/Y may be negative via '+-5')
    let s = value.trim().replace(' ', "");
    let (wh, rest) = s
        .split_once('+')
        .ok_or_else(|| anyhow::anyhow!("invalid rect geometry: {value:?} (expected WxH+X+Y)"))?;
    let (x_s, y_s) = rest
        .split_once('+')
        .ok_or_else(|| anyhow::anyhow!("invalid rect geometry: {value:?} (expected WxH+X+Y)"))?;
    let (w_s, h_s) = wh
        .split_once('x')
        .ok_or_else(|| anyhow::anyhow!("invalid rect geometry: {value:?} (expected WxH+X+Y)"))?;

    let w = w_s
        .parse::<i32>()
        .map_err(|_| anyhow::anyhow!("invalid rect geometry: {value:?} (expected WxH+X+Y)"))?;
    let h = h_s
        .parse::<i32>()
        .map_err(|_| anyhow::anyhow!("invalid rect geometry: {value:?} (expected WxH+X+Y)"))?;
    let x = x_s
        .parse::<i32>()
        .map_err(|_| anyhow::anyhow!("invalid rect geometry: {value:?} (expected WxH+X+Y)"))?;
    let y = y_s
        .parse::<i32>()
        .map_err(|_| anyhow::anyhow!("invalid rect geometry: {value:?} (expected WxH+X+Y)"))?;

    if w <= 0 || h <= 0 {
        return Err(anyhow::anyhow!(
            "invalid rect geometry: {value:?} (W and H must be > 0)"
        ));
    }

    Ok((w, h, x, y))
}

pub fn parse_size(value: &str) -> anyhow::Result<(i32, i32)> {
    let s = value.trim().replace(' ', "");
    let (w, h) = s
        .split_once('x')
        .ok_or_else(|| anyhow::anyhow!("invalid size: {value:?} (expected WxH)"))?;
    let w = w
        .parse::<i32>()
        .map_err(|_| anyhow::anyhow!("invalid size: {value:?} (expected WxH)"))?;
    let h = h
        .parse::<i32>()
        .map_err(|_| anyhow::anyhow!("invalid size: {value:?} (expected WxH)"))?;
    if w <= 0 || h <= 0 {
        return Err(anyhow::anyhow!(
            "invalid size: {value:?} (W and H must be > 0)"
        ));
    }
    Ok((w, h))
}

fn ext_normalize(path: &Path) -> String {
    let ext = path
        .extension()
        .map(|x| x.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if ext == "jpeg" {
        return "jpg".to_string();
    }
    ext
}

fn output_supports_alpha(ext: &str) -> bool {
    matches!(ext, "png" | "webp")
}

fn is_non_alpha_format(ext: &str) -> bool {
    ext == "jpg"
}

fn require_background(reason: &str) -> anyhow::Result<String> {
    Ok(format!("{reason} (provide --background <color>)"))
}

fn build_magick_cmd(toolchain: &Toolchain, input_path: &Path) -> anyhow::Result<Vec<String>> {
    if let Some(magick) = &toolchain.magick {
        let mut cmd = magick.clone();
        cmd.push(input_path.to_string_lossy().to_string());
        return Ok(cmd);
    }
    if let Some(convert) = &toolchain.convert {
        let mut cmd = convert.clone();
        cmd.push(input_path.to_string_lossy().to_string());
        return Ok(cmd);
    }
    Err(anyhow::anyhow!("no ImageMagick backend available"))
}

fn run_one_magick(cmd: &[String], dry_run: bool) -> anyhow::Result<(i32, String, String)> {
    if dry_run {
        return Ok((0, String::new(), String::new()));
    }
    let out = run_output_argv(cmd)?;
    Ok((
        out.status.code().unwrap_or(1),
        out.stdout_lossy(),
        out.stderr_lossy(),
    ))
}

fn compute_resize_box(
    orig_w: i32,
    orig_h: i32,
    scale: Option<f64>,
    width: Option<i32>,
    height: Option<i32>,
    aspect: Option<(i32, i32)>,
    fit: Option<&str>,
) -> anyhow::Result<(i32, i32, Option<String>, bool)> {
    if let Some(scale) = scale {
        if width.is_some() || height.is_some() || aspect.is_some() || fit.is_some() {
            return Err(anyhow::anyhow!(
                "--scale is mutually exclusive with --width/--height/--aspect/--fit"
            ));
        }
        if scale <= 0.0 {
            return Err(anyhow::anyhow!("--scale must be > 0"));
        }
        let tw = ((orig_w as f64) * scale).round().max(1.0) as i32;
        let th = ((orig_h as f64) * scale).round().max(1.0) as i32;
        return Ok((tw, th, None, false));
    }

    if aspect.is_none() {
        if width.is_none() && height.is_none() {
            return Err(anyhow::anyhow!(
                "resize requires one of: --scale, --width, --height, or --aspect + size"
            ));
        }

        if let (Some(w), None) = (width, height) {
            if w <= 0 {
                return Err(anyhow::anyhow!("--width must be > 0"));
            }
            let th = ((orig_h as f64) * (w as f64 / orig_w as f64))
                .round()
                .max(1.0) as i32;
            if fit.is_some() {
                return Err(anyhow::anyhow!(
                    "--fit is only valid when a target box is fully specified"
                ));
            }
            return Ok((w, th, None, false));
        }

        if let (None, Some(h)) = (width, height) {
            if h <= 0 {
                return Err(anyhow::anyhow!("--height must be > 0"));
            }
            let tw = ((orig_w as f64) * (h as f64 / orig_h as f64))
                .round()
                .max(1.0) as i32;
            if fit.is_some() {
                return Err(anyhow::anyhow!(
                    "--fit is only valid when a target box is fully specified"
                ));
            }
            return Ok((tw, h, None, false));
        }

        let (w, h) = (width.unwrap(), height.unwrap());
        if w <= 0 || h <= 0 {
            return Err(anyhow::anyhow!("--width/--height must be > 0"));
        }
        let Some(fit) = fit else {
            return Err(anyhow::anyhow!(
                "when using --width + --height, --fit contain|cover|stretch is required"
            ));
        };
        if !matches!(fit, "contain" | "cover" | "stretch") {
            return Err(anyhow::anyhow!(
                "--fit must be one of: contain, cover, stretch"
            ));
        }
        return Ok((w, h, Some(fit.to_string()), true));
    }

    let (aw, ah) = aspect.unwrap();
    if width.is_none() && height.is_none() {
        return Err(anyhow::anyhow!(
            "when using --aspect, you must also specify --width or --height"
        ));
    }
    let Some(fit) = fit else {
        return Err(anyhow::anyhow!(
            "when using --aspect, --fit contain|cover|stretch is required"
        ));
    };
    if !matches!(fit, "contain" | "cover" | "stretch") {
        return Err(anyhow::anyhow!(
            "--fit must be one of: contain, cover, stretch"
        ));
    }

    if let (Some(w), Some(h)) = (width, height) {
        let wa = w as f64 / h as f64;
        let aa = aw as f64 / ah as f64;
        if (wa - aa).abs() > 1e-6 {
            return Err(anyhow::anyhow!("--width/--height must match --aspect"));
        }
        return Ok((w, h, Some(fit.to_string()), true));
    }

    if let Some(w) = width {
        if w <= 0 {
            return Err(anyhow::anyhow!("--width must be > 0"));
        }
        let h = ((w as f64) * (ah as f64 / aw as f64)).round().max(1.0) as i32;
        return Ok((w, h, Some(fit.to_string()), true));
    }

    let h = height.unwrap();
    if h <= 0 {
        return Err(anyhow::anyhow!("--height must be > 0"));
    }
    let w = ((h as f64) * (aw as f64 / ah as f64)).round().max(1.0) as i32;
    Ok((w, h, Some(fit.to_string()), true))
}

fn run_capture(argv: &[String], fallback_msg: &str) -> anyhow::Result<()> {
    let out = run_output_argv(argv)?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = out.stderr_lossy().trim().to_string();
    let stdout = out.stdout_lossy().trim().to_string();
    let msg = if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        fallback_msg.to_string()
    };
    Err(anyhow::anyhow!("{msg}"))
}

fn run_output_argv(argv: &[String]) -> std::io::Result<common_process::ProcessOutput> {
    let program = argv.first().expect("argv[0]").as_str();
    let args: Vec<&str> = argv[1..].iter().map(String::as_str).collect();
    common_process::run_output(program, &args)
}

fn run_djpeg_cjpeg_pipeline(djpeg_cmd: &[String], cjpeg_cmd: &[String]) -> anyhow::Result<()> {
    let mut p1 = Command::new(&djpeg_cmd[0]);
    p1.args(&djpeg_cmd[1..])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut p1_child = p1.spawn()?;

    let p1_stdout = p1_child.stdout.take().expect("djpeg stdout");
    let mut p1_stderr = p1_child.stderr.take().expect("djpeg stderr");

    let stderr_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = p1_stderr.read_to_end(&mut buf);
        buf
    });

    let mut p2 = Command::new(&cjpeg_cmd[0]);
    p2.args(&cjpeg_cmd[1..])
        .stdin(p1_stdout)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let p2_out = p2.spawn()?.wait_with_output()?;

    let p1_status = p1_child.wait()?;
    let p1_stderr_bytes = stderr_handle.join().unwrap_or_default();

    if !p1_status.success() {
        let msg = String::from_utf8_lossy(&p1_stderr_bytes)
            .trim()
            .to_string()
            .if_empty("djpeg failed");
        return Err(anyhow::anyhow!("{msg}"));
    }
    if !p2_out.status.success() {
        let msg = String::from_utf8_lossy(&p2_out.stderr)
            .trim()
            .to_string()
            .if_empty("cjpeg failed");
        return Err(anyhow::anyhow!("{msg}"));
    }
    Ok(())
}

trait IfEmpty {
    fn if_empty(self, fallback: &str) -> String;
}

impl IfEmpty for String {
    fn if_empty(self, fallback: &str) -> String {
        if self.trim().is_empty() {
            fallback.to_string()
        } else {
            self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    #[test]
    fn compute_resize_box_scale_rules_and_minimums() {
        let err = compute_resize_box(100, 50, Some(1.0), Some(10), None, None, None).unwrap_err();
        assert!(err.to_string().contains("--scale is mutually exclusive"));

        let err = compute_resize_box(100, 50, Some(0.0), None, None, None, None).unwrap_err();
        assert!(err.to_string().contains("--scale must be > 0"));

        let (tw, th, fit, uses_box) =
            compute_resize_box(100, 50, Some(0.0001), None, None, None, None).unwrap();
        assert_eq!(tw, 1);
        assert_eq!(th, 1);
        assert_eq!(fit, None);
        assert!(!uses_box);
    }

    #[test]
    fn compute_resize_box_width_and_height_variants() {
        let (tw, th, fit, uses_box) =
            compute_resize_box(200, 100, None, Some(100), None, None, None).unwrap();
        assert_eq!((tw, th), (100, 50));
        assert_eq!(fit, None);
        assert!(!uses_box);

        let err =
            compute_resize_box(200, 100, None, Some(100), None, None, Some("contain")).unwrap_err();
        assert!(err.to_string().contains("--fit is only valid"));

        let (tw, th, fit, uses_box) =
            compute_resize_box(200, 100, None, None, Some(25), None, None).unwrap();
        assert_eq!((tw, th), (50, 25));
        assert_eq!(fit, None);
        assert!(!uses_box);
    }

    #[test]
    fn compute_resize_box_requires_fit_for_box_and_validates_fit_values() {
        let err = compute_resize_box(200, 100, None, Some(100), Some(100), None, None).unwrap_err();
        assert!(
            err.to_string()
                .contains("--fit contain|cover|stretch is required")
        );

        let err = compute_resize_box(200, 100, None, Some(100), Some(100), None, Some("bad"))
            .unwrap_err();
        assert!(err.to_string().contains("--fit must be one of"));

        let (tw, th, fit, uses_box) =
            compute_resize_box(200, 100, None, Some(100), Some(100), None, Some("cover")).unwrap();
        assert_eq!((tw, th), (100, 100));
        assert_eq!(fit, Some("cover".to_string()));
        assert!(uses_box);
    }

    #[test]
    fn compute_resize_box_aspect_validates_and_derives_size() {
        let err = compute_resize_box(200, 100, None, None, None, Some((16, 9)), None).unwrap_err();
        assert!(
            err.to_string()
                .contains("must also specify --width or --height")
        );

        let err = compute_resize_box(
            200,
            100,
            None,
            Some(100),
            Some(100),
            Some((16, 9)),
            Some("contain"),
        )
        .unwrap_err();
        assert!(err.to_string().contains("must match --aspect"));

        let (tw, th, fit, uses_box) = compute_resize_box(
            200,
            100,
            None,
            Some(160),
            None,
            Some((16, 9)),
            Some("cover"),
        )
        .unwrap();
        assert_eq!((tw, th), (160, 90));
        assert_eq!(fit, Some("cover".to_string()));
        assert!(uses_box);
    }

    #[test]
    fn parse_aspect_and_size_validate_inputs() {
        assert_eq!(parse_aspect("16:9").unwrap(), (16, 9));
        assert_eq!(parse_aspect_opt(None).unwrap(), None);
        assert_eq!(parse_aspect_opt(Some("4:3")).unwrap(), Some((4, 3)));

        let err = parse_aspect("0:9").unwrap_err();
        assert!(err.to_string().contains("W and H must be > 0"));
        let err = parse_aspect("oops").unwrap_err();
        assert!(err.to_string().contains("expected W:H"));

        assert_eq!(parse_size("120x60").unwrap(), (120, 60));
        let err = parse_size("0x1").unwrap_err();
        assert!(err.to_string().contains("W and H must be > 0"));
    }

    #[test]
    fn parse_geometry_accepts_offsets_and_validates() {
        assert_eq!(parse_geometry("100x50+10+-5").unwrap(), (100, 50, 10, -5));
        let err = parse_geometry("100x0+0+0").unwrap_err();
        assert!(err.to_string().contains("W and H must be > 0"));
    }

    #[test]
    fn extension_helpers_normalize_and_classify() {
        assert_eq!(ext_normalize(Path::new("photo.JPEG")), "jpg");
        assert_eq!(ext_normalize(Path::new("photo.PNG")), "png");
        assert_eq!(ext_normalize(Path::new("photo")), "");
        assert!(output_supports_alpha("png"));
        assert!(output_supports_alpha("webp"));
        assert!(!output_supports_alpha("jpg"));
        assert!(is_non_alpha_format("jpg"));
        assert!(!is_non_alpha_format("png"));
        assert!(
            require_background("needs color")
                .unwrap()
                .contains("needs color")
        );
    }

    #[test]
    fn validate_output_mode_enforces_constraints() {
        let err =
            validate_output_mode(Operation::Info, Some("out.png"), None, false, false).unwrap_err();
        assert!(err.to_string().contains("info does not write outputs"));
        assert!(
            validate_output_mode(Operation::Info, None, None, false, false)
                .unwrap()
                .is_none()
        );

        let err = validate_output_mode(Operation::Convert, None, None, false, false).unwrap_err();
        assert!(
            err.to_string()
                .contains("must specify exactly one output mode")
        );

        let err = validate_output_mode(
            Operation::Convert,
            Some("out.png"),
            Some("out"),
            false,
            false,
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("must specify exactly one output mode")
        );

        let err = validate_output_mode(Operation::Convert, None, None, true, false).unwrap_err();
        assert!(
            err.to_string()
                .contains("--in-place is destructive and requires --yes")
        );

        let out = validate_output_mode(Operation::Convert, Some("out.png"), None, false, false)
            .unwrap()
            .unwrap();
        assert_eq!(out.mode, "out");
        assert!(out.out.as_ref().unwrap().ends_with("out.png"));

        let out = validate_output_mode(Operation::Convert, None, Some("out"), false, false)
            .unwrap()
            .unwrap();
        assert_eq!(out.mode, "out_dir");
        assert!(out.out_dir.as_ref().unwrap().ends_with("out"));

        let out = validate_output_mode(Operation::Convert, None, None, true, true)
            .unwrap()
            .unwrap();
        assert_eq!(out.mode, "in_place");
    }

    #[test]
    fn expand_inputs_resolves_files_dirs_and_globs() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.png"), "img").unwrap();
        std::fs::write(root.join("b.jpg"), "img").unwrap();
        let nested = root.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("c.png"), "img").unwrap();

        let inputs = vec![root.to_string_lossy().to_string()];
        let out = expand_inputs(&inputs, false, &["*.png".to_string()]).unwrap();
        let mut names = out
            .iter()
            .filter_map(|p| p.file_name())
            .map(|p| p.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(names, vec!["a.png"]);

        let out = expand_inputs(&inputs, true, &["*.png".to_string()]).unwrap();
        let mut names = out
            .iter()
            .filter_map(|p| p.file_name())
            .map(|p| p.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(names, vec!["a.png", "c.png"]);

        let out = expand_inputs(
            &[root.join("a.png").to_string_lossy().to_string()],
            false,
            &[],
        )
        .unwrap();
        assert_eq!(out.len(), 1);

        let err = expand_inputs(&[], false, &[]).unwrap_err();
        assert!(err.to_string().contains("missing --in"));

        let err = expand_inputs(
            &[root.join("missing.png").to_string_lossy().to_string()],
            false,
            &[],
        )
        .unwrap_err();
        assert!(err.to_string().contains("input not found"));

        let err = expand_inputs(&inputs, false, &["*.gif".to_string()]).unwrap_err();
        assert!(
            err.to_string()
                .contains("no input files resolved from --in/--glob")
        );
    }

    #[test]
    fn build_magick_cmd_selects_backend_or_errors() {
        let base = Toolchain {
            magick: None,
            convert: None,
            identify: vec!["identify".to_string()],
            cwebp: None,
            dwebp: None,
            cjpeg: None,
            djpeg: None,
        };

        let toolchain = Toolchain {
            magick: Some(vec!["magick".to_string()]),
            ..base.clone()
        };
        let cmd = build_magick_cmd(&toolchain, Path::new("in.png")).unwrap();
        assert_eq!(cmd[0], "magick");
        assert!(cmd.iter().any(|c| c.ends_with("in.png")));

        let toolchain = Toolchain {
            magick: None,
            convert: Some(vec!["convert".to_string()]),
            ..base.clone()
        };
        let cmd = build_magick_cmd(&toolchain, Path::new("in.png")).unwrap();
        assert_eq!(cmd[0], "convert");

        let err = build_magick_cmd(&base, Path::new("in.png")).unwrap_err();
        assert!(err.to_string().contains("no ImageMagick backend"));
    }

    #[test]
    fn run_one_magick_is_noop_in_dry_run() {
        let cmd = vec!["convert".to_string(), "in.png".to_string()];
        let (rc, stdout, stderr) = run_one_magick(&cmd, true).unwrap();
        assert_eq!(rc, 0);
        assert!(stdout.is_empty());
        assert!(stderr.is_empty());
    }
}
