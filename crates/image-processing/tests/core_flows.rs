mod common;

use std::fs;

use pretty_assertions::assert_eq;

#[test]
fn from_svg_supports_png_webp_svg_outputs_without_external_binaries() {
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

        if to == "svg" {
            let svg = fs::read_to_string(dir.path().join(&out_rel)).unwrap();
            assert!(svg.contains("<svg"), "svg output missing <svg: {svg}");
        }

        assert!(
            dir.path().join(&out_rel).exists(),
            "missing output: {out_rel}"
        );
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
fn svg_validate_writes_sanitized_output_and_summary_artifact() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::write(
        dir.path().join("valid.svg"),
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16">
<rect x="1" y="1" width="14" height="14" fill="#0f62fe"/>
</svg>"##,
    )
    .unwrap();

    let stub = common::make_stub_dir();
    let path_s = stub.path().to_string_lossy().to_string();
    let envs = [("PATH", path_s.as_str())];

    let out = common::run_image_processing(
        dir.path(),
        &[
            "svg-validate",
            "--in",
            "valid.svg",
            "--out",
            "out/valid.cleaned.svg",
            "--json",
        ],
        &envs,
    );
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);

    let v: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(v["operation"], "svg-validate");
    assert_eq!(v["backend"], "rust:svg-validate");
    assert_eq!(v["source"]["mode"], "svg_validate");
    assert_eq!(v["items"][0]["status"], "ok");
    assert_eq!(v["items"][0]["output_info"]["format"], "SVG");

    let run_id = v["run_id"].as_str().unwrap();
    let summary_json = dir
        .path()
        .join("out/image-processing/runs")
        .join(run_id)
        .join("summary.json");
    assert!(summary_json.exists(), "missing {}", summary_json.display());
    assert!(dir.path().join("out/valid.cleaned.svg").exists());
}
