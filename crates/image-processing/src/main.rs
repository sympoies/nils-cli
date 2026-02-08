use clap::{CommandFactory, Parser};
use nils_term::progress::{Progress, ProgressFinish, ProgressOptions};
use std::process;

mod cli;
mod model;
mod processing;
mod report;
mod toolchain;
mod util;

use cli::{Cli, Operation};

fn main() {
    process::exit(run());
}

fn run() -> i32 {
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(err) => {
            let code = err.exit_code();
            let _ = err.print();
            return code;
        }
    };

    if let Err(e) = validate(&cli) {
        usage_error(&e.message);
    }

    let toolchain = match toolchain::detect_toolchain() {
        Ok(t) => t,
        Err(err) => {
            eprintln!("image-processing: error: {err}");
            return 1;
        }
    };

    let repo_root = util::find_repo_root();

    let inputs = match processing::expand_inputs(&cli.inputs, cli.recursive, &cli.glob) {
        Ok(v) => v,
        Err(e) => usage_error(&e.message),
    };

    let output_mode = match processing::validate_output_mode(
        cli.subcommand,
        cli.out.as_deref(),
        cli.out_dir.as_deref(),
        cli.in_place,
        cli.yes,
    ) {
        Ok(v) => v,
        Err(e) => usage_error(&e.message),
    };

    // Parse aspect/crop selectors (usage errors).
    let resize_aspect = if cli.subcommand == Operation::Resize {
        match processing::parse_aspect_opt(cli.aspect.as_deref()) {
            Ok(v) => v,
            Err(err) => usage_error(&err.to_string()),
        }
    } else {
        None
    };
    let crop_aspect = if cli.subcommand == Operation::Crop {
        match processing::parse_aspect_opt(cli.aspect.as_deref()) {
            Ok(v) => v,
            Err(err) => usage_error(&err.to_string()),
        }
    } else {
        None
    };

    let crop_rect = if cli.subcommand == Operation::Crop {
        match cli.rect.as_deref() {
            Some(s) => match processing::parse_geometry(s) {
                Ok(v) => Some(v),
                Err(err) => usage_error(&err.to_string()),
            },
            None => None,
        }
    } else {
        None
    };

    let crop_size = if cli.subcommand == Operation::Crop {
        match cli.size.as_deref() {
            Some(s) => match processing::parse_size(s) {
                Ok(v) => Some(v),
                Err(err) => usage_error(&err.to_string()),
            },
            None => None,
        }
    } else {
        None
    };

    if cli.subcommand == Operation::Crop {
        let count = [cli.rect.is_some(), cli.size.is_some(), cli.aspect.is_some()]
            .into_iter()
            .filter(|x| *x)
            .count();
        if count != 1 {
            usage_error("crop requires exactly one of: --rect, --size, or --aspect");
        }
    }

    // Preflight (usage) validations that depend on inputs.
    if cli.subcommand == Operation::Convert
        && cli.to.as_deref() == Some("jpg")
        && cli.background.is_none()
    {
        for p in &inputs {
            let info = toolchain::probe_image(&toolchain, p);
            if info.alpha.unwrap_or(false) {
                usage_error("alpha input cannot be converted to JPEG without a background (provide --background <color>)");
            }
        }
    }

    // Run dir
    let mut run_dir: Option<std::path::PathBuf> = None;
    if cli.json || cli.report {
        let run_id = util::now_run_id();
        let p = repo_root
            .join("out")
            .join("image-processing")
            .join("runs")
            .join(run_id);
        if let Err(err) = std::fs::create_dir_all(&p) {
            eprintln!("image-processing: error: {err}");
            return 1;
        }
        run_dir = Some(p);
    }

    let progress = Progress::new(
        inputs.len() as u64,
        ProgressOptions::default().with_finish(ProgressFinish::Leave),
    );

    let summary = match processing::process_items(processing::ProcessArgs {
        toolchain: &toolchain,
        repo_root: &repo_root,
        run_dir: run_dir.as_deref(),
        progress,
        subcommand: cli.subcommand,
        inputs: &inputs,
        output_mode: output_mode.as_ref(),
        overwrite: cli.overwrite,
        dry_run: cli.dry_run,
        auto_orient_enabled: cli.auto_orient,
        strip_metadata: cli.strip_metadata,
        background: cli.background.as_deref(),
        report_enabled: cli.report,
        json_enabled: cli.json,
        convert_to: cli.to.as_deref(),
        quality: cli.quality,
        resize_scale: cli.scale,
        resize_width: if cli.subcommand == Operation::Resize {
            cli.width
        } else {
            None
        },
        resize_height: if cli.subcommand == Operation::Resize {
            cli.height
        } else {
            None
        },
        resize_aspect,
        resize_fit: cli.fit.as_deref(),
        no_pre_upscale: cli.no_pre_upscale,
        rotate_degrees: cli.degrees,
        crop_rect,
        crop_size,
        crop_aspect,
        crop_gravity: &cli.gravity,
        pad_width: if cli.subcommand == Operation::Pad {
            cli.width
        } else {
            None
        },
        pad_height: if cli.subcommand == Operation::Pad {
            cli.height
        } else {
            None
        },
        pad_gravity: &cli.gravity,
        optimize_lossless: cli.lossless,
        optimize_progressive: cli.progressive,
    }) {
        Ok(s) => s,
        Err(err) => {
            if err.downcast_ref::<util::UsageError>().is_some() {
                usage_error(&err.to_string());
            }
            eprintln!("image-processing: error: {err}");
            return 1;
        }
    };

    if cli.json {
        match serde_json::to_string(&summary) {
            Ok(s) => {
                println!("{s}");
            }
            Err(err) => {
                eprintln!("image-processing: error: {err}");
                return 1;
            }
        }
    } else {
        println!("operation: {}", summary.operation);
        if let Some(rd) = run_dir.as_deref() {
            println!("run_dir: {}", util::maybe_relpath(rd, &repo_root));
        }
        for item in &summary.items {
            let outp = item.output_path.as_deref().unwrap_or("None");
            println!("- {}: {} -> {}", item.status, item.input_path, outp);
        }
    }

    let any_error = summary.items.iter().any(|i| i.status == "error");
    if any_error {
        1
    } else {
        0
    }
}

fn validate(cli: &Cli) -> Result<(), util::UsageError> {
    let forbid = |flag: &str| -> Result<(), util::UsageError> {
        Err(util::UsageError {
            message: format!("{} does not support {flag}", cli.subcommand.as_str()),
        })
    };

    if cli.subcommand != Operation::Convert && cli.to.is_some() {
        forbid("--to")?;
    }
    if !matches!(cli.subcommand, Operation::Convert | Operation::Optimize) && cli.quality.is_some()
    {
        forbid("--quality")?;
    }

    if cli.subcommand != Operation::Resize {
        if cli.scale.is_some() {
            forbid("--scale")?;
        }
        if cli.fit.is_some() {
            forbid("--fit")?;
        }
        if cli.no_pre_upscale {
            forbid("--no-pre-upscale")?;
        }
    }

    if !matches!(cli.subcommand, Operation::Resize | Operation::Pad) {
        if cli.width.is_some() {
            forbid("--width")?;
        }
        if cli.height.is_some() {
            forbid("--height")?;
        }
    }

    if !matches!(cli.subcommand, Operation::Resize | Operation::Crop) && cli.aspect.is_some() {
        forbid("--aspect")?;
    }

    if cli.subcommand != Operation::Rotate && cli.degrees.is_some() {
        forbid("--degrees")?;
    }

    if !matches!(cli.subcommand, Operation::Crop | Operation::Pad) {
        if cli.rect.is_some() {
            forbid("--rect")?;
        }
        if cli.size.is_some() {
            forbid("--size")?;
        }
        if cli.gravity != "center" {
            forbid("--gravity")?;
        }
    }

    if cli.subcommand != Operation::Optimize {
        if cli.lossless {
            forbid("--lossless")?;
        }
        if !cli.progressive {
            forbid("--no-progressive")?;
        }
    }

    // Subcommand-specific required args.
    if cli.subcommand == Operation::Convert {
        if cli.to.is_none() {
            return Err(util::UsageError {
                message: "convert requires --to png|jpg|webp".to_string(),
            });
        }
        if let Some(to) = cli.to.as_deref() {
            if !model::SUPPORTED_CONVERT_TARGETS.contains(&to) {
                return Err(util::UsageError {
                    message: "convert --to must be one of: png|jpg|webp".to_string(),
                });
            }
        }
    }

    if cli.subcommand == Operation::Rotate && cli.degrees.is_none() {
        return Err(util::UsageError {
            message: "rotate requires --degrees".to_string(),
        });
    }

    if cli.subcommand == Operation::Pad && (cli.width.is_none() || cli.height.is_none()) {
        return Err(util::UsageError {
            message: "pad requires --width and --height".to_string(),
        });
    }

    if cli.subcommand == Operation::Resize {
        if let Some(fit) = cli.fit.as_deref() {
            if !matches!(fit, "contain" | "cover" | "stretch") {
                return Err(util::UsageError {
                    message: "resize --fit must be one of: contain, cover, stretch".to_string(),
                });
            }
        }
        if cli.aspect.is_some() && cli.fit.is_none() {
            return Err(util::UsageError {
                message: "resize with --aspect requires --fit contain|cover|stretch".to_string(),
            });
        }
        if cli.width.is_some() && cli.height.is_some() && cli.fit.is_none() {
            return Err(util::UsageError {
                message: "resize with --width + --height requires --fit contain|cover|stretch"
                    .to_string(),
            });
        }
    }

    Ok(())
}

fn usage_error(msg: &str) -> ! {
    let mut cmd = Cli::command();
    let usage = cmd.render_usage().to_string();
    eprintln!("{usage}");
    eprintln!("image-processing: error: {msg}");
    process::exit(2);
}

#[cfg(test)]
mod tests {
    use super::{validate, Cli, Operation};

    fn base_cli(op: Operation) -> Cli {
        Cli {
            subcommand: op,
            inputs: vec!["in.png".to_string()],
            recursive: false,
            glob: Vec::new(),
            out: Some("out.png".to_string()),
            out_dir: None,
            in_place: false,
            yes: false,
            overwrite: false,
            dry_run: true,
            json: false,
            report: false,
            auto_orient: true,
            strip_metadata: false,
            background: None,
            to: None,
            quality: None,
            scale: None,
            width: None,
            height: None,
            aspect: None,
            fit: None,
            no_pre_upscale: false,
            degrees: None,
            rect: None,
            size: None,
            gravity: "center".to_string(),
            lossless: false,
            progressive: true,
        }
    }

    #[test]
    fn validate_convert_requires_supported_to_values() {
        let mut cli = base_cli(Operation::Convert);
        let err = validate(&cli).expect_err("missing --to should fail");
        assert!(err.to_string().contains("convert requires --to"));

        cli.to = Some("gif".to_string());
        let err = validate(&cli).expect_err("unsupported --to should fail");
        assert!(err.to_string().contains("must be one of: png|jpg|webp"));

        cli.to = Some("webp".to_string());
        assert!(validate(&cli).is_ok());
    }

    #[test]
    fn validate_rejects_convert_only_flags_on_other_subcommands() {
        let mut cli = base_cli(Operation::Rotate);
        cli.to = Some("png".to_string());
        let err = validate(&cli).expect_err("rotate should reject --to");
        assert!(err.to_string().contains("rotate does not support --to"));

        let mut cli = base_cli(Operation::Flip);
        cli.quality = Some(90);
        let err = validate(&cli).expect_err("flip should reject --quality");
        assert!(err.to_string().contains("flip does not support --quality"));
    }

    #[test]
    fn validate_resize_requires_fit_for_box_and_aspect() {
        let mut cli = base_cli(Operation::Resize);
        cli.to = None;
        cli.width = Some(100);
        cli.height = Some(50);
        let err = validate(&cli).expect_err("missing fit for width+height should fail");
        assert!(err.to_string().contains("requires --fit"));

        cli.fit = Some("contain".to_string());
        assert!(validate(&cli).is_ok());

        cli.aspect = Some("16:9".to_string());
        cli.fit = None;
        let err = validate(&cli).expect_err("aspect without fit should fail");
        assert!(err.to_string().contains("with --aspect requires --fit"));
    }

    #[test]
    fn validate_rotate_and_pad_require_specific_arguments() {
        let mut rotate = base_cli(Operation::Rotate);
        rotate.to = None;
        let err = validate(&rotate).expect_err("rotate requires degrees");
        assert!(err.to_string().contains("rotate requires --degrees"));
        rotate.degrees = Some(90);
        assert!(validate(&rotate).is_ok());

        let mut pad = base_cli(Operation::Pad);
        pad.to = None;
        let err = validate(&pad).expect_err("pad requires dimensions");
        assert!(err
            .to_string()
            .contains("pad requires --width and --height"));
        pad.width = Some(640);
        pad.height = Some(480);
        assert!(validate(&pad).is_ok());
    }

    #[test]
    fn validate_rejects_crop_and_optimize_flag_misuse() {
        let mut crop = base_cli(Operation::Crop);
        crop.to = None;
        crop.rect = Some("1x1+0+0".to_string());
        crop.gravity = "south".to_string();
        assert!(validate(&crop).is_ok());

        let mut flip = base_cli(Operation::Flip);
        flip.rect = Some("1x1+0+0".to_string());
        let err = validate(&flip).expect_err("flip should reject crop-only flag");
        assert!(err.to_string().contains("flip does not support --rect"));

        let mut optimize = base_cli(Operation::Optimize);
        optimize.to = None;
        optimize.progressive = false;
        assert!(validate(&optimize).is_ok());

        let mut non_opt = base_cli(Operation::Resize);
        non_opt.to = None;
        non_opt.progressive = false;
        let err = validate(&non_opt).expect_err("non-optimize should reject --no-progressive");
        assert!(err
            .to_string()
            .contains("resize does not support --no-progressive"));
    }
}
