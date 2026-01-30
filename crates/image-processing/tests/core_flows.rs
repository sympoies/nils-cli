mod common;

use std::fs;

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
