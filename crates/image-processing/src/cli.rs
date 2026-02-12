use clap::{ArgAction, Parser, ValueEnum};

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum Operation {
    Info,
    SvgValidate,
    AutoOrient,
    Convert,
    Resize,
    Rotate,
    Crop,
    Pad,
    Flip,
    Flop,
    Optimize,
}

impl Operation {
    pub fn as_str(&self) -> &'static str {
        match self {
            Operation::Info => "info",
            Operation::SvgValidate => "svg-validate",
            Operation::AutoOrient => "auto-orient",
            Operation::Convert => "convert",
            Operation::Resize => "resize",
            Operation::Rotate => "rotate",
            Operation::Crop => "crop",
            Operation::Pad => "pad",
            Operation::Flip => "flip",
            Operation::Flop => "flop",
            Operation::Optimize => "optimize",
        }
    }
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FromSvgValidationRule {
    pub when: &'static str,
    pub expect: &'static str,
}

#[allow(dead_code)]
pub const FROM_SVG_VALIDATION_MATRIX: [FromSvgValidationRule; 6] = [
    FromSvgValidationRule {
        when: "subcommand=convert && --from-svg",
        expect: "requires --out and forbids --out-dir/--in-place",
    },
    FromSvgValidationRule {
        when: "subcommand=convert && --from-svg",
        expect: "forbids --in/--recursive/--glob",
    },
    FromSvgValidationRule {
        when: "subcommand=convert && --from-svg",
        expect: "requires --to and supports only png|webp|svg",
    },
    FromSvgValidationRule {
        when: "subcommand=svg-validate",
        expect: "requires exactly one --in and explicit --out",
    },
    FromSvgValidationRule {
        when: "subcommand=svg-validate",
        expect: "forbids --from-svg/--recursive/--glob/--in-place",
    },
    FromSvgValidationRule {
        when: "subcommand=svg-validate",
        expect: "emits deterministic sanitized svg for identical input",
    },
];

#[derive(Debug, Parser)]
#[command(
    name = "image-processing",
    about = "Batch image transformations with svg-source and svg-validation flows.",
    after_help = "Notes:\n  - Output-producing subcommands require exactly one output mode: --out, --out-dir, or --in-place (with --yes).\n  - convert --from-svg uses the Rust SVG backend and requires --out + --to png|webp|svg.\n  - svg-validate sanitizes a single svg input and requires --in + --out.\n  - Use --json for machine-readable output (stdout JSON only; logs go to stderr).\n"
)]
pub struct Cli {
    #[arg(value_enum)]
    pub subcommand: Operation,

    #[arg(long = "in", action = ArgAction::Append, default_value = None)]
    pub inputs: Vec<String>,
    #[arg(long)]
    pub recursive: bool,
    #[arg(long, action = ArgAction::Append, default_value = None)]
    pub glob: Vec<String>,

    #[arg(long = "from-svg")]
    pub from_svg: Option<String>,

    #[arg(long)]
    pub out: Option<String>,
    #[arg(long = "out-dir")]
    pub out_dir: Option<String>,
    #[arg(long = "in-place")]
    pub in_place: bool,

    #[arg(long)]
    pub yes: bool,
    #[arg(long)]
    pub overwrite: bool,
    #[arg(long = "dry-run")]
    pub dry_run: bool,
    #[arg(long)]
    pub json: bool,
    #[arg(long)]
    pub report: bool,

    #[arg(
        long = "no-auto-orient",
        action = ArgAction::SetFalse,
        default_value_t = true
    )]
    pub auto_orient: bool,
    #[arg(long = "strip-metadata")]
    pub strip_metadata: bool,
    #[arg(long)]
    pub background: Option<String>,

    // convert / optimize / convert --from-svg
    #[arg(long = "to")]
    pub to: Option<String>,
    #[arg(long)]
    pub quality: Option<i32>,

    // resize / pad
    #[arg(long)]
    pub scale: Option<f64>,
    #[arg(long)]
    pub width: Option<i32>,
    #[arg(long)]
    pub height: Option<i32>,
    #[arg(long)]
    pub aspect: Option<String>,
    #[arg(long)]
    pub fit: Option<String>,
    #[arg(long = "no-pre-upscale")]
    pub no_pre_upscale: bool,

    // rotate
    #[arg(long)]
    pub degrees: Option<i32>,

    // crop
    #[arg(long)]
    pub rect: Option<String>,
    #[arg(long)]
    pub size: Option<String>,
    #[arg(long, default_value = "center")]
    pub gravity: String,

    // optimize
    #[arg(long)]
    pub lossless: bool,
    #[arg(
        long = "no-progressive",
        action = ArgAction::SetFalse,
        default_value_t = true
    )]
    pub progressive: bool,
}
