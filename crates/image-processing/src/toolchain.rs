use crate::model::ImageInfo;
use nils_common::process::find_in_path;
use std::path::Path;
use std::process::Command;

pub const RUST_FROM_SVG_BACKEND: &str = "rust:resvg";
pub const RUST_SVG_VALIDATE_BACKEND: &str = "rust:svg-validate";

#[derive(Clone, Debug)]
pub struct Toolchain {
    pub magick: Option<Vec<String>>,
    pub convert: Option<Vec<String>>,
    pub identify: Vec<String>,
    pub cwebp: Option<String>,
    pub dwebp: Option<String>,
    pub cjpeg: Option<String>,
    pub djpeg: Option<String>,
}

impl Toolchain {
    pub fn primary_backend(&self) -> &'static str {
        if self.magick.is_some() {
            return "imagemagick:magick";
        }
        if self.convert.is_some() {
            return "imagemagick:convert";
        }
        "imagemagick:unknown"
    }
}

pub fn operation_requires_imagemagick(operation: &str, from_svg_mode: bool) -> bool {
    if operation == "svg-validate" {
        return false;
    }
    if operation == "convert" && from_svg_mode {
        return false;
    }
    true
}

pub fn backend_for_operation(
    operation: &str,
    toolchain: Option<&Toolchain>,
    from_svg_mode: bool,
) -> &'static str {
    if operation == "svg-validate" {
        return RUST_SVG_VALIDATE_BACKEND;
    }
    if operation == "convert" && from_svg_mode {
        return RUST_FROM_SVG_BACKEND;
    }

    toolchain
        .map(Toolchain::primary_backend)
        .unwrap_or("imagemagick:unknown")
}

pub fn detect_toolchain() -> anyhow::Result<Toolchain> {
    let magick = find_in_path("magick");
    let convert = find_in_path("convert");
    let identify = find_in_path("identify");

    let (magick_cmd, convert_cmd, identify_cmd) = if let Some(magick) = magick {
        let magick_s = magick.to_string_lossy().to_string();
        (
            Some(vec![magick_s.clone()]),
            None,
            vec![magick_s, "identify".to_string()],
        )
    } else if let (Some(convert), Some(identify)) = (convert, identify) {
        (
            None,
            Some(vec![convert.to_string_lossy().to_string()]),
            vec![identify.to_string_lossy().to_string()],
        )
    } else {
        anyhow::bail!("missing ImageMagick (need `magick` or both `convert` + `identify`)");
    };

    Ok(Toolchain {
        magick: magick_cmd,
        convert: convert_cmd,
        identify: identify_cmd,
        cwebp: find_in_path("cwebp").map(|p| p.to_string_lossy().to_string()),
        dwebp: find_in_path("dwebp").map(|p| p.to_string_lossy().to_string()),
        cjpeg: find_in_path("cjpeg").map(|p| p.to_string_lossy().to_string()),
        djpeg: find_in_path("djpeg").map(|p| p.to_string_lossy().to_string()),
    })
}

pub fn probe_image(toolchain: &Toolchain, path: &Path) -> ImageInfo {
    let mut info = ImageInfo {
        size_bytes: std::fs::metadata(path).ok().map(|m| m.len()),
        ..Default::default()
    };

    let fmt = "%m|%w|%h|%[channels]|%[exif:Orientation]";
    let mut argv: Vec<String> = Vec::new();
    argv.extend(toolchain.identify.iter().cloned());
    argv.extend([
        "-ping".to_string(),
        "-format".to_string(),
        fmt.to_string(),
        path.to_string_lossy().to_string(),
    ]);

    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    let out = match cmd.output() {
        Ok(o) => o,
        Err(_) => return info,
    };
    if !out.status.success() {
        return info;
    }

    let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if raw.is_empty() {
        return info;
    }

    let first = raw.lines().next().unwrap_or("").trim();
    if first.is_empty() {
        return info;
    }

    let parts: Vec<&str> = first.split('|').collect();
    if let Some(fmt) = parts.first().copied().filter(|s| !s.trim().is_empty()) {
        info.format = Some(fmt.trim().to_string());
    }
    if parts.len() >= 3
        && let (Ok(w), Ok(h)) = (
            parts[1].trim().parse::<i32>(),
            parts[2].trim().parse::<i32>(),
        )
    {
        info.width = Some(w);
        info.height = Some(h);
    }
    if parts.len() >= 4 {
        let ch = parts[3].trim();
        if !ch.is_empty() {
            let ch_s = ch.to_string();
            let alpha = ch_s.to_lowercase().contains('a');
            info.channels = Some(ch_s);
            info.alpha = Some(alpha);
        }
    }
    if parts.len() >= 5 {
        let exif = parts[4].trim();
        if !exif.is_empty() {
            info.exif_orientation = Some(exif.to_string());
        }
    }

    info
}
