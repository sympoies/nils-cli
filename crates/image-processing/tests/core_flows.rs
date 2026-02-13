mod common;

use std::fs;

use pretty_assertions::assert_eq;

#[test]
fn info_json_emits_schema_and_writes_summary_json() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::write(dir.path().join("a.png"), "img").unwrap();

    let stub = common::make_stub_dir();
    common::write_exe(stub.path(), "identify", common::identify_stub_script());
    common::write_exe(stub.path(), "convert", common::convert_stub_script());

    let path_s = stub.path().to_string_lossy().to_string();
    let envs = [("PATH", path_s.as_str())];

    let out = common::run_image_processing(dir.path(), &["info", "--in", "a.png", "--json"], &envs);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);

    let v: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(v["schema_version"], 1);
    assert_eq!(v["operation"], "info");
    assert!(v["items"].is_array());
    assert_eq!(v["items"].as_array().unwrap().len(), 1);
    assert!(v["items"][0]["output_path"].is_null());

    let run_id = v["run_id"].as_str().unwrap();
    let summary_json = dir
        .path()
        .join("out/image-processing/runs")
        .join(run_id)
        .join("summary.json");
    assert!(summary_json.exists(), "missing {}", summary_json.display());
}

#[test]
fn info_plain_text_keeps_legacy_output_shape() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::write(dir.path().join("a.png"), "img").unwrap();

    let stub = common::make_stub_dir();
    common::write_exe(stub.path(), "identify", common::identify_stub_script());
    common::write_exe(stub.path(), "convert", common::convert_stub_script());

    let path_s = stub.path().to_string_lossy().to_string();
    let envs = [("PATH", path_s.as_str())];

    let out = common::run_image_processing(dir.path(), &["info", "--in", "a.png"], &envs);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert!(
        out.stdout.contains("operation: info"),
        "stdout: {}",
        out.stdout
    );
    assert!(out.stdout.contains("- ok:"), "stdout: {}", out.stdout);
    assert!(out.stdout.contains("-> None"), "stdout: {}", out.stdout);
}

#[test]
fn convert_dry_run_does_not_write_output_file() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::write(dir.path().join("a.png"), "img").unwrap();

    let stub = common::make_stub_dir();
    common::write_exe(stub.path(), "identify", common::identify_stub_script());
    common::write_exe(stub.path(), "convert", common::convert_stub_script());

    let path_s = stub.path().to_string_lossy().to_string();
    let envs = [("PATH", path_s.as_str())];

    let out = common::run_image_processing(
        dir.path(),
        &[
            "convert",
            "--in",
            "a.png",
            "--to",
            "webp",
            "--out",
            "out/a.webp",
            "--dry-run",
            "--json",
        ],
        &envs,
    );
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert!(
        !dir.path().join("out/a.webp").exists(),
        "dry-run should not write output"
    );
}

#[test]
fn resize_includes_default_pre_upscale() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::write(dir.path().join("a.png"), "img").unwrap();

    let stub = common::make_stub_dir();
    common::write_exe(stub.path(), "identify", common::identify_stub_script());
    common::write_exe(stub.path(), "convert", common::convert_stub_script());

    let path_s = stub.path().to_string_lossy().to_string();
    let envs = [("PATH", path_s.as_str())];

    let out = common::run_image_processing(
        dir.path(),
        &[
            "resize",
            "--in",
            "a.png",
            "--scale",
            "2",
            "--out",
            "out/a.png",
            "--json",
        ],
        &envs,
    );
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    let v: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    let cmds = v["commands"].as_array().unwrap();
    let joined = cmds
        .iter()
        .map(|x| x.as_str().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        joined.contains("200%"),
        "expected pre-upscale in commands, got:\n{joined}"
    );
}

#[test]
fn optimize_uses_cjpeg_pipeline_when_available() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::write(dir.path().join("a.jpg"), "jpg").unwrap();

    let stub = common::make_stub_dir();
    common::write_exe(stub.path(), "identify", common::identify_stub_script());
    common::write_exe(stub.path(), "convert", common::convert_stub_script());
    common::write_exe(stub.path(), "djpeg", common::djpeg_stub_script());
    common::write_exe(stub.path(), "cjpeg", common::cjpeg_stub_script());

    let path_s = stub.path().to_string_lossy().to_string();
    let envs = [("PATH", path_s.as_str())];

    let out = common::run_image_processing(
        dir.path(),
        &["optimize", "--in", "a.jpg", "--out", "out/a.jpg", "--json"],
        &envs,
    );
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);

    let v: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    let cmds = v["commands"].as_array().unwrap();
    let joined = cmds
        .iter()
        .map(|x| x.as_str().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        joined.contains("|") && joined.contains("cjpeg"),
        "expected djpeg|cjpeg pipeline, got:\n{joined}"
    );
    assert!(dir.path().join("out/a.jpg").exists());
}

#[test]
fn optimize_uses_cwebp_when_available() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::write(dir.path().join("a.webp"), "webp").unwrap();

    let stub = common::make_stub_dir();
    common::write_exe(stub.path(), "identify", common::identify_stub_script());
    common::write_exe(stub.path(), "convert", common::convert_stub_script());
    common::write_exe(stub.path(), "dwebp", common::dwebp_stub_script());
    common::write_exe(stub.path(), "cwebp", common::cwebp_stub_script());

    let path_s = stub.path().to_string_lossy().to_string();
    let envs = [("PATH", path_s.as_str())];

    let out = common::run_image_processing(
        dir.path(),
        &[
            "optimize",
            "--in",
            "a.webp",
            "--out",
            "out/a.webp",
            "--json",
        ],
        &envs,
    );
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);

    let v: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    let cmds = v["commands"].as_array().unwrap();
    let joined = cmds
        .iter()
        .map(|x| x.as_str().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        joined.contains("dwebp") && joined.contains("cwebp"),
        "expected dwebp + cwebp commands, got:\n{joined}"
    );
    assert!(dir.path().join("out/a.webp").exists());
}

#[test]
fn magick_backend_is_preferred_when_present() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::write(dir.path().join("a.png"), "img").unwrap();

    let stub = common::make_stub_dir();
    common::write_exe(stub.path(), "magick", common::magick_stub_script());
    common::write_exe(stub.path(), "identify", common::identify_stub_script());
    common::write_exe(stub.path(), "convert", common::convert_stub_script());

    let path_s = stub.path().to_string_lossy().to_string();
    let envs = [("PATH", path_s.as_str())];

    let out = common::run_image_processing(dir.path(), &["info", "--in", "a.png", "--json"], &envs);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    let v: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(v["backend"], "imagemagick:magick");
}

#[test]
fn from_svg_supports_png_webp_svg_outputs_without_imagemagick() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::write(
        dir.path().join("icon.svg"),
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32">
<rect x="2" y="2" width="28" height="28" rx="6" fill="#0f62fe"/>
<path d="M9 16h14" stroke="#ffffff" stroke-width="3" stroke-linecap="round"/>
</svg>"##,
    )
    .unwrap();

    let stub = common::make_stub_dir();
    let path_s = stub.path().to_string_lossy().to_string();
    let envs = [("PATH", path_s.as_str())];

    for (to, expected_format) in [("png", "PNG"), ("webp", "WEBP"), ("svg", "SVG")] {
        let out_rel = format!("out/icon.{to}");
        let args = [
            "convert".to_string(),
            "--from-svg".to_string(),
            "icon.svg".to_string(),
            "--to".to_string(),
            to.to_string(),
            "--out".to_string(),
            out_rel.clone(),
            "--json".to_string(),
        ];
        let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        let out = common::run_image_processing(dir.path(), &arg_refs, &envs);
        assert_eq!(out.code, 0, "to={to}, stderr: {}", out.stderr);

        let v: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
        assert_eq!(v["operation"], "convert");
        assert_eq!(v["backend"], "rust:resvg");
        assert_eq!(v["source"]["mode"], "from_svg");
        assert_eq!(v["source"]["from_svg"], "icon.svg");
        assert_eq!(v["items"].as_array().unwrap().len(), 1);
        assert_eq!(v["items"][0]["status"], "ok");
        assert_eq!(v["items"][0]["output_path"], out_rel);
        assert_eq!(v["items"][0]["output_info"]["format"], expected_format);
        assert_eq!(v["items"][0]["output_info"]["width"], 32);
        assert_eq!(v["items"][0]["output_info"]["height"], 32);
        assert!(
            v["items"][0]["output_info"]["size_bytes"]
                .as_u64()
                .unwrap_or_default()
                > 0
        );
        assert!(
            v["items"][0]["output_info"]["alpha"].as_bool().is_some(),
            "expected alpha metadata for {to}"
        );

        if to == "svg" {
            assert!(v["items"][0]["output_info"]["channels"].is_null());
            let svg = fs::read_to_string(dir.path().join(&out_rel)).unwrap();
            assert!(svg.contains("<svg"), "svg output missing <svg: {svg}");
        } else {
            assert!(
                v["items"][0]["output_info"]["channels"].as_str().is_some(),
                "expected raster channels metadata for {to}"
            );
        }

        assert!(
            dir.path().join(&out_rel).exists(),
            "missing output: {out_rel}"
        );
        let run_id = v["run_id"].as_str().unwrap();
        let summary_json = dir
            .path()
            .join("out/image-processing/runs")
            .join(run_id)
            .join("summary.json");
        assert!(summary_json.exists(), "missing {}", summary_json.display());
    }
}

#[test]
fn from_svg_supports_explicit_raster_dimensions() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::write(
        dir.path().join("icon.svg"),
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 80 60" width="80" height="60">
<rect x="4" y="4" width="72" height="52" rx="8" fill="#0f62fe"/>
</svg>"##,
    )
    .unwrap();

    let stub = common::make_stub_dir();
    let path_s = stub.path().to_string_lossy().to_string();
    let envs = [("PATH", path_s.as_str())];

    let width_only = common::run_image_processing(
        dir.path(),
        &[
            "convert",
            "--from-svg",
            "icon.svg",
            "--to",
            "png",
            "--out",
            "out/icon-width.png",
            "--width",
            "512",
            "--json",
        ],
        &envs,
    );
    assert_eq!(width_only.code, 0, "stderr: {}", width_only.stderr);
    let width_only_json: serde_json::Value = serde_json::from_str(&width_only.stdout).unwrap();
    assert_eq!(width_only_json["items"][0]["output_info"]["width"], 512);
    assert_eq!(width_only_json["items"][0]["output_info"]["height"], 384);

    let exact_box = common::run_image_processing(
        dir.path(),
        &[
            "convert",
            "--from-svg",
            "icon.svg",
            "--to",
            "png",
            "--out",
            "out/icon-box.png",
            "--width",
            "512",
            "--height",
            "512",
            "--json",
        ],
        &envs,
    );
    assert_eq!(exact_box.code, 0, "stderr: {}", exact_box.stderr);
    let exact_box_json: serde_json::Value = serde_json::from_str(&exact_box.stdout).unwrap();
    assert_eq!(exact_box_json["items"][0]["output_info"]["width"], 512);
    assert_eq!(exact_box_json["items"][0]["output_info"]["height"], 512);
}

#[test]
fn from_svg_still_runs_when_legacy_operations_lack_imagemagick() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::write(dir.path().join("a.png"), "img").unwrap();
    fs::write(
        dir.path().join("icon.svg"),
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32">
<rect x="4" y="4" width="24" height="24" rx="4" fill="#0f62fe"/>
</svg>"##,
    )
    .unwrap();

    let stub = common::make_stub_dir();
    let path_s = stub.path().to_string_lossy().to_string();
    let envs = [("PATH", path_s.as_str())];

    let info_out =
        common::run_image_processing(dir.path(), &["info", "--in", "a.png", "--json"], &envs);
    assert_eq!(info_out.code, 1);
    assert!(
        info_out.stderr.contains("missing ImageMagick"),
        "stderr: {}",
        info_out.stderr
    );

    let from_svg_out = common::run_image_processing(
        dir.path(),
        &[
            "convert",
            "--from-svg",
            "icon.svg",
            "--to",
            "png",
            "--out",
            "out/info.png",
            "--json",
        ],
        &envs,
    );
    assert_eq!(from_svg_out.code, 0, "stderr: {}", from_svg_out.stderr);
    let v: serde_json::Value = serde_json::from_str(&from_svg_out.stdout).unwrap();
    assert_eq!(v["operation"], "convert");
    assert_eq!(v["backend"], "rust:resvg");
    assert_eq!(v["source"]["mode"], "from_svg");
    assert!(dir.path().join("out/info.png").exists());
}
