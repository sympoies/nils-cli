use clap::{ArgAction, Parser, ValueEnum};

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum Operation {
    Convert,
    SvgValidate,
}

impl Operation {
    pub fn as_str(&self) -> &'static str {
        match self {
            Operation::Convert => "convert",
            Operation::SvgValidate => "svg-validate",
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
pub const FROM_SVG_VALIDATION_MATRIX: [FromSvgValidationRule; 5] = [
    FromSvgValidationRule {
        when: "subcommand=convert",
        expect: "requires --from-svg + --to png|webp|svg + --out",
    },
    FromSvgValidationRule {
        when: "subcommand=convert",
        expect: "forbids --in",
    },
    FromSvgValidationRule {
        when: "subcommand=convert",
        expect: "accepts optional --width/--height for png|webp outputs",
    },
    FromSvgValidationRule {
        when: "subcommand=svg-validate",
        expect: "requires exactly one --in and explicit --out",
    },
    FromSvgValidationRule {
        when: "subcommand=svg-validate",
        expect: "forbids --from-svg/--to/--width/--height",
    },
];

#[derive(Debug, Parser)]
#[command(
    name = "image-processing",
    version,
    about = "Validate SVG inputs and convert trusted SVG to raster/vector outputs.",
    after_help = "Notes:\n  - convert requires --from-svg + --to png|webp|svg + --out.\n  - convert supports optional --width/--height for png|webp output sizing.\n  - svg-validate requires exactly one --in + --out.\n  - Use --json for machine-readable output (stdout JSON only; logs go to stderr).\n"
)]
pub struct Cli {
    #[arg(value_enum)]
    pub subcommand: Operation,

    #[arg(long = "in", action = ArgAction::Append, default_value = None)]
    pub inputs: Vec<String>,

    #[arg(long = "from-svg")]
    pub from_svg: Option<String>,

    #[arg(long)]
    pub out: Option<String>,

    #[arg(long)]
    pub overwrite: bool,
    #[arg(long = "dry-run")]
    pub dry_run: bool,
    #[arg(long)]
    pub json: bool,
    #[arg(long)]
    pub report: bool,

    #[arg(long = "to")]
    pub to: Option<String>,

    #[arg(long)]
    pub width: Option<i32>,
    #[arg(long)]
    pub height: Option<i32>,
}
