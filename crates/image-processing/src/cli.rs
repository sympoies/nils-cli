use clap::{ArgAction, Parser, ValueEnum};

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum Operation {
    Info,
    Generate,
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
            Operation::Generate => "generate",
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum GeneratePreset {
    Info,
    Success,
    Warning,
    Error,
    Help,
}

#[allow(dead_code)]
pub const GENERATE_DEFAULT_TO: &str = "png";
#[allow(dead_code)]
pub const GENERATE_DEFAULT_SIZE: &str = "64";
#[allow(dead_code)]
pub const GENERATE_DEFAULT_FG: &str = "#ffffff";
#[allow(dead_code)]
pub const GENERATE_DEFAULT_BG: &str = "#0f62fe";
#[allow(dead_code)]
pub const GENERATE_DEFAULT_STROKE_WIDTH: &str = "0";
#[allow(dead_code)]
pub const GENERATE_DEFAULT_PADDING: &str = "0";

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GenerateValidationRule {
    pub when: &'static str,
    pub expect: &'static str,
}

#[allow(dead_code)]
pub const GENERATE_VALIDATION_MATRIX: [GenerateValidationRule; 8] = [
    GenerateValidationRule {
        when: "subcommand=generate",
        expect: "--preset is required and repeatable",
    },
    GenerateValidationRule {
        when: "subcommand=generate",
        expect: "--to defaults to png and only accepts png|webp|svg",
    },
    GenerateValidationRule {
        when: "subcommand=generate && variants=1",
        expect: "--out is required",
    },
    GenerateValidationRule {
        when: "subcommand=generate && variants>1",
        expect: "--out-dir is required",
    },
    GenerateValidationRule {
        when: "subcommand=generate",
        expect: "--in-place is forbidden",
    },
    GenerateValidationRule {
        when: "subcommand=generate",
        expect: "--in/--recursive/--glob are forbidden",
    },
    GenerateValidationRule {
        when: "subcommand=generate",
        expect: "--size accepts integer pixels (fallback default when omitted)",
    },
    GenerateValidationRule {
        when: "subcommand=generate",
        expect: "--fg/--bg/--stroke accept color strings",
    },
];

#[derive(Debug, Parser)]
#[command(
    name = "image-processing",
    about = "Batch image transformations plus deterministic generate presets.",
    after_help = "Notes:\n  - Output-producing subcommands require exactly one output mode: --out, --out-dir, or --in-place (with --yes).\n  - generate requires --preset and does not read --in or allow --in-place.\n  - Use --json for machine-readable output (stdout JSON only; logs go to stderr).\n"
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

    // generate
    #[arg(
        long = "preset",
        value_enum,
        action = ArgAction::Append,
        required_if_eq("subcommand", "generate")
    )]
    pub presets: Vec<GeneratePreset>,
    #[arg(long = "fg")]
    pub fg: Option<String>,
    #[arg(long = "bg")]
    pub bg: Option<String>,
    #[arg(long)]
    pub stroke: Option<String>,
    #[arg(long = "stroke-width")]
    pub stroke_width: Option<f64>,
    #[arg(long)]
    pub padding: Option<f64>,

    // convert / optimize / generate
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
    // crop: WxH, generate: integer pixels
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
