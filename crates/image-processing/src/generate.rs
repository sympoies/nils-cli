use crate::cli::{
    GENERATE_DEFAULT_BG, GENERATE_DEFAULT_FG, GENERATE_DEFAULT_PADDING, GENERATE_DEFAULT_SIZE,
    GENERATE_DEFAULT_STROKE_WIDTH, GENERATE_DEFAULT_TO, GeneratePreset,
};
use crate::model::{ImageInfo, OutputMode};
use crate::util;
use image::codecs::png::{CompressionType as PngCompression, FilterType as PngFilter, PngEncoder};
use image::codecs::webp::WebPEncoder;
use image::{ExtendedColorType, ImageEncoder};
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;
use svg::node::element::path::Data;
use svg::node::element::{Circle, Line, Path as SvgPath, Polygon, Polyline, Rectangle};

pub const SUPPORTED_GENERATE_TARGETS: [&str; 3] = ["png", "webp", "svg"];
const GENERATE_ALPHA_POLICY: &str = "preserve-rgba";
const GENERATE_WEBP_MODE: &str = "lossless-vp8l";
const GENERATE_PNG_COMPRESSION: PngCompression = PngCompression::Best;
const GENERATE_PNG_FILTER: PngFilter = PngFilter::NoFilter;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GenerateOutputFormat {
    Png,
    Webp,
    Svg,
}

impl GenerateOutputFormat {
    pub fn parse(raw: Option<&str>) -> anyhow::Result<Self> {
        let to = raw.unwrap_or(GENERATE_DEFAULT_TO);
        match to {
            "png" => Ok(Self::Png),
            "webp" => Ok(Self::Webp),
            "svg" => Ok(Self::Svg),
            _ => Err(util::usage_err(format!(
                "generate --to must be one of: {}",
                SUPPORTED_GENERATE_TARGETS.join("|")
            ))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Webp => "webp",
            Self::Svg => "svg",
        }
    }
}

#[derive(Clone, Debug)]
pub struct GenerateStyle {
    pub fg: String,
    pub bg: String,
    pub stroke: Option<String>,
    pub stroke_width: f64,
    pub padding: f64,
}

/// Stable request shape for the Rust-native generate backend.
#[derive(Clone, Debug)]
pub struct GeneratePlanRequest<'a> {
    pub output_mode: Option<&'a OutputMode>,
    pub presets: &'a [GeneratePreset],
    pub to: Option<&'a str>,
    pub size: Option<&'a str>,
    pub fg: Option<&'a str>,
    pub bg: Option<&'a str>,
    pub stroke: Option<&'a str>,
    pub stroke_width: Option<f64>,
    pub padding: Option<f64>,
}

/// Stable plan object Sprint 2 can extend with variant paths and write semantics.
#[derive(Clone, Debug)]
pub struct GeneratePlan {
    pub output_mode: &'static str,
    pub output_format: GenerateOutputFormat,
    pub size_px: u32,
    pub presets: Vec<GeneratePreset>,
    pub style: GenerateStyle,
}

#[derive(Clone, Debug, Default)]
pub struct GenerateDispatchResult {
    pub commands: Vec<String>,
    pub output_info: Option<ImageInfo>,
}

#[derive(Clone, Copy, Debug)]
struct Geometry {
    inset: f32,
    span: f32,
    corner_radius: f32,
    glyph_stroke: f32,
}

impl Geometry {
    fn new(size_px: u32, padding: f64) -> anyhow::Result<Self> {
        let size = size_px as f32;
        let inset = padding as f32;
        if (inset * 2.0) >= size {
            return Err(util::usage_err(
                "generate --padding must be less than half of --size",
            ));
        }
        let span = size - (inset * 2.0);
        Ok(Self {
            inset,
            span,
            corner_radius: (span * 0.16).max(0.0),
            glyph_stroke: (span * 0.1).max(1.0),
        })
    }

    fn point(self, x: f32, y: f32) -> (f32, f32) {
        (self.inset + (self.span * x), self.inset + (self.span * y))
    }

    fn radius(self, ratio: f32) -> f32 {
        (self.span * ratio).max(0.5)
    }
}

pub fn plan_generate(request: GeneratePlanRequest<'_>) -> anyhow::Result<GeneratePlan> {
    let output_mode = request
        .output_mode
        .ok_or_else(|| util::usage_err("generate requires an explicit output mode"))?;
    if output_mode.mode == "in_place" {
        return Err(util::usage_err("generate does not support --in-place"));
    }
    if request.presets.is_empty() {
        return Err(util::usage_err(
            "generate requires at least one --preset value",
        ));
    }

    let output_format = GenerateOutputFormat::parse(request.to)?;
    let size_px = parse_size_px(request.size.unwrap_or(GENERATE_DEFAULT_SIZE))?;
    let fg = normalize_hex_color(request.fg.unwrap_or(GENERATE_DEFAULT_FG), "--fg")?;
    let bg = normalize_hex_color(request.bg.unwrap_or(GENERATE_DEFAULT_BG), "--bg")?;
    let stroke = request
        .stroke
        .map(|raw| normalize_hex_color(raw, "--stroke"))
        .transpose()?;

    let stroke_width = request
        .stroke_width
        .unwrap_or_else(|| GENERATE_DEFAULT_STROKE_WIDTH.parse().unwrap_or(0.0));
    if !stroke_width.is_finite() {
        return Err(util::usage_err("generate --stroke-width must be finite"));
    }
    if stroke_width < 0.0 {
        return Err(util::usage_err("generate --stroke-width must be >= 0"));
    }
    let padding = request
        .padding
        .unwrap_or_else(|| GENERATE_DEFAULT_PADDING.parse().unwrap_or(0.0));
    if !padding.is_finite() {
        return Err(util::usage_err("generate --padding must be finite"));
    }
    if padding < 0.0 {
        return Err(util::usage_err("generate --padding must be >= 0"));
    }
    let _ = Geometry::new(size_px, padding)?;

    Ok(GeneratePlan {
        output_mode: output_mode.mode,
        output_format,
        size_px,
        presets: request.presets.to_vec(),
        style: GenerateStyle {
            fg,
            bg,
            stroke,
            stroke_width,
            padding,
        },
    })
}

pub fn dispatch_generate(
    plan: &GeneratePlan,
    preset: GeneratePreset,
    output_path: &Path,
    dry_run: bool,
) -> anyhow::Result<GenerateDispatchResult> {
    let svg_doc = build_preset_svg(plan, preset)?;
    let commands = vec![build_generate_command(plan, preset, output_path, dry_run)];

    if dry_run {
        return Ok(GenerateDispatchResult {
            commands,
            output_info: None,
        });
    }

    util::ensure_parent_dir(output_path, false)?;
    let tmp = util::safe_write_path(output_path, false);

    let output_alpha = match plan.output_format {
        GenerateOutputFormat::Svg => {
            std::fs::write(&tmp, svg_doc.as_bytes())?;
            svg_uses_alpha(plan)
        }
        GenerateOutputFormat::Png | GenerateOutputFormat::Webp => {
            let pixmap = rasterize_svg(&svg_doc, plan.size_px)?;
            let rgba = demultiply_rgba(&pixmap);
            let has_alpha = rgba_has_alpha(&rgba);
            match plan.output_format {
                GenerateOutputFormat::Png => write_png(&tmp, plan.size_px, &rgba)?,
                GenerateOutputFormat::Webp => write_webp(&tmp, plan.size_px, &rgba)?,
                GenerateOutputFormat::Svg => unreachable!("matched above"),
            }
            has_alpha
        }
    };

    util::atomic_replace(&tmp, output_path, false)?;
    let output_info = generated_output_info(
        plan.output_format,
        plan.size_px,
        output_path,
        Some(output_alpha),
    )?;

    Ok(GenerateDispatchResult {
        commands,
        output_info: Some(output_info),
    })
}

fn parse_size_px(raw: &str) -> anyhow::Result<u32> {
    let size_px = raw
        .trim()
        .parse::<u32>()
        .map_err(|_| util::usage_err("generate --size must be a positive integer"))?;
    if size_px == 0 {
        return Err(util::usage_err("generate --size must be > 0"));
    }
    Ok(size_px)
}

pub fn preset_label(preset: GeneratePreset) -> &'static str {
    match preset {
        GeneratePreset::Info => "info",
        GeneratePreset::Success => "success",
        GeneratePreset::Warning => "warning",
        GeneratePreset::Error => "error",
        GeneratePreset::Help => "help",
    }
}

pub fn variant_file_name(plan: &GeneratePlan, preset: GeneratePreset) -> String {
    let stroke_token = plan
        .style
        .stroke
        .as_ref()
        .map(|s| color_token(s))
        .unwrap_or_else(|| "none".to_string());
    format!(
        "{}__size-{}__fg-{}__bg-{}__stroke-{}__sw-{}__pad-{}.{}",
        preset_label(preset),
        plan.size_px,
        color_token(&plan.style.fg),
        color_token(&plan.style.bg),
        stroke_token,
        canonical_decimal(plan.style.stroke_width),
        canonical_decimal(plan.style.padding),
        plan.output_format.as_str()
    )
}

fn build_preset_svg(plan: &GeneratePlan, preset: GeneratePreset) -> anyhow::Result<String> {
    let geometry = Geometry::new(plan.size_px, plan.style.padding)?;
    let mut doc = svg::Document::new()
        .set("viewBox", (0, 0, plan.size_px, plan.size_px))
        .set("width", plan.size_px)
        .set("height", plan.size_px)
        .add(background_rect(plan, geometry));

    doc = match preset {
        GeneratePreset::Info => doc
            .add(outline_circle(plan, geometry, 0.5, 0.45, 0.28))
            .add(glyph_line(plan, geometry, 0.5, 0.41, 0.5, 0.64))
            .add(filled_circle(plan, geometry, 0.5, 0.77, 0.04)),
        GeneratePreset::Success => doc.add(glyph_polyline(
            plan,
            geometry,
            &[(0.22, 0.55), (0.42, 0.73), (0.78, 0.35)],
        )),
        GeneratePreset::Warning => doc
            .add(glyph_polygon(
                plan,
                geometry,
                &[(0.5, 0.16), (0.84, 0.82), (0.16, 0.82)],
            ))
            .add(glyph_line(plan, geometry, 0.5, 0.40, 0.5, 0.61))
            .add(filled_circle(plan, geometry, 0.5, 0.72, 0.04)),
        GeneratePreset::Error => doc
            .add(glyph_line(plan, geometry, 0.28, 0.28, 0.72, 0.72))
            .add(glyph_line(plan, geometry, 0.72, 0.28, 0.28, 0.72)),
        GeneratePreset::Help => doc
            .add(glyph_question_path(plan, geometry))
            .add(filled_circle(plan, geometry, 0.52, 0.80, 0.04)),
    };

    Ok(doc.to_string())
}

fn background_rect(plan: &GeneratePlan, geometry: Geometry) -> Rectangle {
    let mut rect = Rectangle::new()
        .set("x", geometry.inset)
        .set("y", geometry.inset)
        .set("width", geometry.span)
        .set("height", geometry.span)
        .set("rx", geometry.corner_radius)
        .set("ry", geometry.corner_radius)
        .set("fill", plan.style.bg.clone());
    if let Some(stroke) = &plan.style.stroke {
        rect = rect
            .set("stroke", stroke.clone())
            .set("stroke-width", plan.style.stroke_width);
    }
    rect
}

fn filled_circle(plan: &GeneratePlan, geometry: Geometry, x: f32, y: f32, r: f32) -> Circle {
    let (cx, cy) = geometry.point(x, y);
    Circle::new()
        .set("cx", cx)
        .set("cy", cy)
        .set("r", geometry.radius(r))
        .set("fill", plan.style.fg.clone())
}

fn outline_circle(plan: &GeneratePlan, geometry: Geometry, x: f32, y: f32, r: f32) -> Circle {
    let (cx, cy) = geometry.point(x, y);
    Circle::new()
        .set("cx", cx)
        .set("cy", cy)
        .set("r", geometry.radius(r))
        .set("fill", "none")
        .set("stroke", plan.style.fg.clone())
        .set("stroke-width", geometry.glyph_stroke * 0.85)
        .set("stroke-linecap", "round")
        .set("stroke-linejoin", "round")
}

fn glyph_line(plan: &GeneratePlan, geometry: Geometry, x1: f32, y1: f32, x2: f32, y2: f32) -> Line {
    let (x1, y1) = geometry.point(x1, y1);
    let (x2, y2) = geometry.point(x2, y2);
    Line::new()
        .set("x1", x1)
        .set("y1", y1)
        .set("x2", x2)
        .set("y2", y2)
        .set("stroke", plan.style.fg.clone())
        .set("stroke-width", geometry.glyph_stroke)
        .set("stroke-linecap", "round")
        .set("stroke-linejoin", "round")
}

fn glyph_polyline(plan: &GeneratePlan, geometry: Geometry, points: &[(f32, f32)]) -> Polyline {
    let points = points
        .iter()
        .map(|(x, y)| {
            let (px, py) = geometry.point(*x, *y);
            format!("{px},{py}")
        })
        .collect::<Vec<_>>()
        .join(" ");
    Polyline::new()
        .set("points", points)
        .set("fill", "none")
        .set("stroke", plan.style.fg.clone())
        .set("stroke-width", geometry.glyph_stroke)
        .set("stroke-linecap", "round")
        .set("stroke-linejoin", "round")
}

fn glyph_polygon(plan: &GeneratePlan, geometry: Geometry, points: &[(f32, f32)]) -> Polygon {
    let points = points
        .iter()
        .map(|(x, y)| {
            let (px, py) = geometry.point(*x, *y);
            format!("{px},{py}")
        })
        .collect::<Vec<_>>()
        .join(" ");
    Polygon::new()
        .set("points", points)
        .set("fill", "none")
        .set("stroke", plan.style.fg.clone())
        .set("stroke-width", geometry.glyph_stroke * 0.88)
        .set("stroke-linecap", "round")
        .set("stroke-linejoin", "round")
}

fn glyph_question_path(plan: &GeneratePlan, geometry: Geometry) -> SvgPath {
    let (sx, sy) = geometry.point(0.35, 0.35);
    let (c1x, c1y) = geometry.point(0.35, 0.24);
    let (c2x, c2y) = geometry.point(0.45, 0.19);
    let (x1, y1) = geometry.point(0.56, 0.19);
    let (c3x, c3y) = geometry.point(0.67, 0.19);
    let (c4x, c4y) = geometry.point(0.75, 0.28);
    let (x2, y2) = geometry.point(0.75, 0.39);
    let (c5x, c5y) = geometry.point(0.75, 0.49);
    let (c6x, c6y) = geometry.point(0.69, 0.56);
    let (x3, y3) = geometry.point(0.61, 0.60);
    let (c7x, c7y) = geometry.point(0.55, 0.63);
    let (c8x, c8y) = geometry.point(0.52, 0.67);
    let (x4, y4) = geometry.point(0.52, 0.72);

    let data = Data::new()
        .move_to((sx, sy))
        .cubic_curve_to((c1x, c1y, c2x, c2y, x1, y1))
        .cubic_curve_to((c3x, c3y, c4x, c4y, x2, y2))
        .cubic_curve_to((c5x, c5y, c6x, c6y, x3, y3))
        .cubic_curve_to((c7x, c7y, c8x, c8y, x4, y4));

    SvgPath::new()
        .set("d", data)
        .set("fill", "none")
        .set("stroke", plan.style.fg.clone())
        .set("stroke-width", geometry.glyph_stroke * 0.85)
        .set("stroke-linecap", "round")
        .set("stroke-linejoin", "round")
}

fn rasterize_svg(svg_doc: &str, size_px: u32) -> anyhow::Result<resvg::tiny_skia::Pixmap> {
    let options = usvg::Options::default();
    let tree = usvg::Tree::from_str(svg_doc, &options)
        .map_err(|err| anyhow::anyhow!("failed to parse generated SVG: {err}"))?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size_px, size_px)
        .ok_or_else(|| anyhow::anyhow!("failed to allocate raster surface for generate"))?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    Ok(pixmap)
}

fn demultiply_rgba(pixmap: &resvg::tiny_skia::Pixmap) -> Vec<u8> {
    let mut rgba = Vec::with_capacity((pixmap.width() * pixmap.height() * 4) as usize);
    for pixel in pixmap.pixels() {
        let c = pixel.demultiply();
        rgba.extend([c.red(), c.green(), c.blue(), c.alpha()]);
    }
    rgba
}

fn write_png(path: &Path, size_px: u32, rgba: &[u8]) -> anyhow::Result<()> {
    let file = File::create(path)?;
    let writer = BufWriter::new(file);
    let encoder =
        PngEncoder::new_with_quality(writer, GENERATE_PNG_COMPRESSION, GENERATE_PNG_FILTER);
    encoder
        .write_image(rgba, size_px, size_px, ExtendedColorType::Rgba8)
        .map_err(|err| anyhow::anyhow!("failed to encode png: {err}"))
}

fn write_webp(path: &Path, size_px: u32, rgba: &[u8]) -> anyhow::Result<()> {
    let file = File::create(path)?;
    let writer = BufWriter::new(file);
    WebPEncoder::new_lossless(writer)
        .encode(rgba, size_px, size_px, ExtendedColorType::Rgba8)
        .map_err(|err| anyhow::anyhow!("failed to encode webp: {err}"))
}

fn generated_output_info(
    output_format: GenerateOutputFormat,
    size_px: u32,
    output_path: &Path,
    alpha: Option<bool>,
) -> anyhow::Result<ImageInfo> {
    let format = match output_format {
        GenerateOutputFormat::Png => "PNG",
        GenerateOutputFormat::Webp => "WEBP",
        GenerateOutputFormat::Svg => "SVG",
    };
    let channels = match output_format {
        GenerateOutputFormat::Svg => None,
        _ => Some(
            if alpha.unwrap_or(false) {
                "rgba"
            } else {
                "rgb"
            }
            .to_string(),
        ),
    };

    Ok(ImageInfo {
        format: Some(format.to_string()),
        width: Some(size_px as i32),
        height: Some(size_px as i32),
        channels,
        alpha,
        exif_orientation: None,
        size_bytes: Some(std::fs::metadata(output_path)?.len()),
    })
}

fn build_generate_command(
    plan: &GeneratePlan,
    preset: GeneratePreset,
    output_path: &Path,
    dry_run: bool,
) -> String {
    let mut argv = vec![
        "generate".to_string(),
        "--preset".to_string(),
        preset_label(preset).to_string(),
        "--to".to_string(),
        plan.output_format.as_str().to_string(),
        "--size".to_string(),
        plan.size_px.to_string(),
        "--fg".to_string(),
        plan.style.fg.clone(),
        "--bg".to_string(),
        plan.style.bg.clone(),
        "--stroke-width".to_string(),
        canonical_decimal(plan.style.stroke_width),
        "--padding".to_string(),
        canonical_decimal(plan.style.padding),
        "--alpha-policy".to_string(),
        GENERATE_ALPHA_POLICY.to_string(),
    ];
    if let Some(stroke) = &plan.style.stroke {
        argv.extend(["--stroke".to_string(), stroke.clone()]);
    }
    match plan.output_format {
        GenerateOutputFormat::Png => argv.extend([
            "--png-compression".to_string(),
            "best".to_string(),
            "--png-filter".to_string(),
            "no-filter".to_string(),
        ]),
        GenerateOutputFormat::Webp => {
            argv.extend(["--webp-mode".to_string(), GENERATE_WEBP_MODE.to_string()])
        }
        GenerateOutputFormat::Svg => argv.extend(["--svg-mode".to_string(), "direct".to_string()]),
    }
    argv.extend([
        "--mode".to_string(),
        plan.output_mode.to_string(),
        "--out".to_string(),
        output_path.to_string_lossy().to_string(),
    ]);
    if dry_run {
        argv.push("--dry-run".to_string());
    }
    util::command_str(&argv)
}

fn rgba_has_alpha(rgba: &[u8]) -> bool {
    rgba.chunks_exact(4).any(|px| px[3] < 255)
}

fn svg_uses_alpha(plan: &GeneratePlan) -> bool {
    color_has_alpha(&plan.style.fg)
        || color_has_alpha(&plan.style.bg)
        || plan
            .style
            .stroke
            .as_ref()
            .is_some_and(|s| color_has_alpha(s))
}

fn color_has_alpha(color: &str) -> bool {
    if let Some(hex) = color.strip_prefix('#')
        && hex.len() == 8
    {
        return !hex[6..8].eq_ignore_ascii_case("ff");
    }
    false
}

fn normalize_hex_color(raw: &str, flag: &str) -> anyhow::Result<String> {
    let value = raw.trim();
    let Some(hex) = value.strip_prefix('#') else {
        return Err(util::usage_err(format!(
            "generate {flag} must be a hex color (#rgb|#rgba|#rrggbb|#rrggbbaa)"
        )));
    };

    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(util::usage_err(format!(
            "generate {flag} must be a hex color (#rgb|#rgba|#rrggbb|#rrggbbaa)"
        )));
    }

    let normalized = match hex.len() {
        3 | 4 => hex
            .chars()
            .flat_map(|c| [c.to_ascii_lowercase(), c.to_ascii_lowercase()])
            .collect::<String>(),
        6 | 8 => hex.to_ascii_lowercase(),
        _ => {
            return Err(util::usage_err(format!(
                "generate {flag} must be a hex color (#rgb|#rgba|#rrggbb|#rrggbbaa)"
            )));
        }
    };
    Ok(format!("#{normalized}"))
}

fn color_token(color: &str) -> String {
    color.trim_start_matches('#').to_ascii_lowercase()
}

fn canonical_decimal(value: f64) -> String {
    let mut out = format!("{value:.6}");
    while out.contains('.') && out.ends_with('0') {
        out.pop();
    }
    if out.ends_with('.') {
        out.pop();
    }
    if out == "-0" {
        return "0".to_string();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::OutputMode;
    use pretty_assertions::assert_eq;

    fn output_mode(mode: &'static str) -> OutputMode {
        OutputMode {
            mode,
            out: Some("out/demo.png".into()),
            out_dir: None,
        }
    }

    #[test]
    fn plan_generate_normalizes_hex_colors() {
        let plan = plan_generate(GeneratePlanRequest {
            output_mode: Some(&output_mode("out")),
            presets: &[GeneratePreset::Info],
            to: Some("svg"),
            size: Some("48"),
            fg: Some("#FFF"),
            bg: Some("#0F62FE"),
            stroke: Some("#abcd"),
            stroke_width: Some(2.0),
            padding: Some(1.5),
        })
        .expect("plan");

        assert_eq!(plan.style.fg, "#ffffff");
        assert_eq!(plan.style.bg, "#0f62fe");
        assert_eq!(plan.style.stroke.as_deref(), Some("#aabbccdd"));
        assert_eq!(plan.size_px, 48);
    }

    #[test]
    fn plan_generate_rejects_invalid_color() {
        let err = plan_generate(GeneratePlanRequest {
            output_mode: Some(&output_mode("out")),
            presets: &[GeneratePreset::Info],
            to: Some("png"),
            size: Some("64"),
            fg: Some("blue"),
            bg: None,
            stroke: None,
            stroke_width: None,
            padding: None,
        })
        .expect_err("invalid color must fail");
        assert!(
            err.to_string()
                .contains("generate --fg must be a hex color")
        );
    }

    #[test]
    fn variant_filename_is_stable() {
        let plan = plan_generate(GeneratePlanRequest {
            output_mode: Some(&output_mode("out_dir")),
            presets: &[GeneratePreset::Warning],
            to: Some("png"),
            size: Some("64"),
            fg: Some("#111111"),
            bg: Some("#ffd166"),
            stroke: None,
            stroke_width: Some(0.0),
            padding: Some(0.0),
        })
        .expect("plan");

        let file = variant_file_name(&plan, GeneratePreset::Warning);
        assert_eq!(
            file,
            "warning__size-64__fg-111111__bg-ffd166__stroke-none__sw-0__pad-0.png"
        );
    }

    #[test]
    fn preset_builders_emit_svg_with_viewbox() {
        let plan = plan_generate(GeneratePlanRequest {
            output_mode: Some(&output_mode("out")),
            presets: &[GeneratePreset::Help],
            to: Some("svg"),
            size: Some("32"),
            fg: None,
            bg: None,
            stroke: None,
            stroke_width: None,
            padding: None,
        })
        .expect("plan");

        for preset in [
            GeneratePreset::Info,
            GeneratePreset::Success,
            GeneratePreset::Warning,
            GeneratePreset::Error,
            GeneratePreset::Help,
        ] {
            let svg = build_preset_svg(&plan, preset).expect("svg");
            assert!(svg.contains("<svg"));
            assert!(svg.contains("viewBox=\"0 0 32 32\""));
        }
    }
}
