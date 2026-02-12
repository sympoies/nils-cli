mod common;

use std::fs;

use pretty_assertions::assert_eq;

#[test]
fn missing_imagemagick_exits_1() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::write(dir.path().join("a.png"), "img").unwrap();

    let stub = common::make_stub_dir();
    let path_s = stub.path().to_string_lossy().to_string();
    let envs = [("PATH", path_s.as_str())];

    let out = common::run_image_processing(dir.path(), &["info", "--in", "a.png", "--json"], &envs);
    assert_eq!(out.code, 1);
    assert!(
        out.stderr.contains("missing ImageMagick"),
        "stderr: {}",
        out.stderr
    );
}

#[test]
fn missing_output_mode_is_usage_error() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::write(dir.path().join("a.png"), "img").unwrap();

    let stub = common::make_stub_dir();
    common::write_exe(stub.path(), "identify", common::identify_stub_script());
    common::write_exe(stub.path(), "convert", common::convert_stub_script());

    let path_s = stub.path().to_string_lossy().to_string();
    let envs = [("PATH", path_s.as_str())];

    let out = common::run_image_processing(
        dir.path(),
        &["convert", "--in", "a.png", "--to", "webp", "--json"],
        &envs,
    );
    assert_eq!(out.code, 2);
    assert!(
        out.stderr.contains("must specify exactly one output mode"),
        "stderr: {}",
        out.stderr
    );
}

// Legacy (non-generate) regression guardrails:
// keep usage exit code and key validation messages stable.
#[test]
fn convert_invalid_target_format_is_usage_error() {
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
            "gif",
            "--out",
            "out/a.gif",
            "--json",
        ],
        &envs,
    );
    assert_eq!(out.code, 2);
    assert!(
        out.stderr
            .contains("convert --to must be one of: png|jpg|webp"),
        "stderr: {}",
        out.stderr
    );
}

#[test]
fn rotate_requires_degrees_is_usage_error() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::write(dir.path().join("a.png"), "img").unwrap();

    let stub = common::make_stub_dir();
    common::write_exe(stub.path(), "identify", common::identify_stub_script());
    common::write_exe(stub.path(), "convert", common::convert_stub_script());

    let path_s = stub.path().to_string_lossy().to_string();
    let envs = [("PATH", path_s.as_str())];

    let out = common::run_image_processing(
        dir.path(),
        &["rotate", "--in", "a.png", "--out", "out/a.png", "--json"],
        &envs,
    );
    assert_eq!(out.code, 2);
    assert!(
        out.stderr.contains("rotate requires --degrees"),
        "stderr: {}",
        out.stderr
    );
}

#[test]
fn info_rejects_convert_only_to_flag() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::write(dir.path().join("a.png"), "img").unwrap();

    let stub = common::make_stub_dir();
    common::write_exe(stub.path(), "identify", common::identify_stub_script());
    common::write_exe(stub.path(), "convert", common::convert_stub_script());

    let path_s = stub.path().to_string_lossy().to_string();
    let envs = [("PATH", path_s.as_str())];

    let out = common::run_image_processing(
        dir.path(),
        &["info", "--in", "a.png", "--to", "webp", "--json"],
        &envs,
    );
    assert_eq!(out.code, 2);
    assert!(
        out.stderr.contains("info does not support --to"),
        "stderr: {}",
        out.stderr
    );
}

#[test]
fn in_place_requires_yes() {
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
            "rotate",
            "--in",
            "a.png",
            "--degrees",
            "90",
            "--in-place",
            "--json",
        ],
        &envs,
    );
    assert_eq!(out.code, 2);
    assert!(
        out.stderr
            .contains("--in-place is destructive and requires --yes"),
        "stderr: {}",
        out.stderr
    );
}

#[test]
fn output_collisions_in_out_dir_are_usage_error() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::write(dir.path().join("a.png"), "img").unwrap();
    fs::write(dir.path().join("a.jpg"), "img").unwrap();

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
            "--in",
            "a.jpg",
            "--to",
            "webp",
            "--out-dir",
            "out",
            "--json",
        ],
        &envs,
    );
    assert_eq!(out.code, 2);
    assert!(
        out.stderr.contains("output collisions detected"),
        "stderr: {}",
        out.stderr
    );
}

#[test]
fn overwrite_is_required_when_output_exists() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::write(dir.path().join("a.png"), "img").unwrap();
    fs::create_dir_all(dir.path().join("out")).unwrap();
    fs::write(dir.path().join("out/a.webp"), "existing").unwrap();

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
            "--json",
        ],
        &envs,
    );
    assert_eq!(out.code, 2);
    assert!(
        out.stderr
            .contains("output exists (pass --overwrite to replace)"),
        "stderr: {}",
        out.stderr
    );
}

#[test]
fn alpha_png_to_jpg_requires_background() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::write(dir.path().join("alpha.png"), "img").unwrap();

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
            "alpha.png",
            "--to",
            "jpg",
            "--out",
            "out/a.jpg",
            "--json",
        ],
        &envs,
    );
    assert_eq!(out.code, 2);
    assert!(
        out.stderr
            .contains("alpha input cannot be converted to JPEG without a background"),
        "stderr: {}",
        out.stderr
    );
}

#[test]
fn pad_jpg_requires_background_is_item_error_exit_1() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::write(dir.path().join("a.jpg"), "img").unwrap();

    let stub = common::make_stub_dir();
    common::write_exe(stub.path(), "identify", common::identify_stub_script());
    common::write_exe(stub.path(), "convert", common::convert_stub_script());

    let path_s = stub.path().to_string_lossy().to_string();
    let envs = [("PATH", path_s.as_str())];

    let out = common::run_image_processing(
        dir.path(),
        &[
            "pad",
            "--in",
            "a.jpg",
            "--width",
            "120",
            "--height",
            "60",
            "--out",
            "out/a.jpg",
            "--json",
        ],
        &envs,
    );
    assert_eq!(out.code, 1, "stderr: {}", out.stderr);
    let v: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(v["items"][0]["status"], "error");
    assert!(
        v["items"][0]["error"]
            .as_str()
            .unwrap_or("")
            .contains("pad requires a background for non-alpha outputs"),
        "error: {}",
        v["items"][0]["error"]
    );
}

#[test]
fn generate_rejects_invalid_input_flags() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::write(dir.path().join("a.png"), "img").unwrap();

    let stub = common::make_stub_dir();
    let path_s = stub.path().to_string_lossy().to_string();
    let envs = [("PATH", path_s.as_str())];

    let with_in = common::run_image_processing(
        dir.path(),
        &[
            "generate",
            "--preset",
            "info",
            "--in",
            "a.png",
            "--to",
            "png",
            "--out",
            "out/info.png",
            "--json",
        ],
        &envs,
    );
    assert_eq!(with_in.code, 2);
    assert!(
        with_in.stderr.contains("generate does not support --in"),
        "stderr: {}",
        with_in.stderr
    );

    let with_in_place = common::run_image_processing(
        dir.path(),
        &[
            "generate",
            "--preset",
            "info",
            "--in-place",
            "--yes",
            "--to",
            "png",
            "--out",
            "out/info.png",
            "--json",
        ],
        &envs,
    );
    assert_eq!(with_in_place.code, 2);
    assert!(
        with_in_place
            .stderr
            .contains("generate does not support --in-place"),
        "stderr: {}",
        with_in_place.stderr
    );
}

#[test]
fn generate_rejects_output_mode_mismatches_for_variant_count() {
    let dir = tempfile::TempDir::new().unwrap();

    let stub = common::make_stub_dir();
    let path_s = stub.path().to_string_lossy().to_string();
    let envs = [("PATH", path_s.as_str())];

    let single_with_out_dir = common::run_image_processing(
        dir.path(),
        &[
            "generate",
            "--preset",
            "info",
            "--to",
            "png",
            "--out-dir",
            "out",
            "--json",
        ],
        &envs,
    );
    assert_eq!(single_with_out_dir.code, 2);
    assert!(
        single_with_out_dir
            .stderr
            .contains("single preset does not support --out-dir"),
        "stderr: {}",
        single_with_out_dir.stderr
    );

    let multi_with_out = common::run_image_processing(
        dir.path(),
        &[
            "generate",
            "--preset",
            "info",
            "--preset",
            "warning",
            "--to",
            "png",
            "--out",
            "out/info.png",
            "--json",
        ],
        &envs,
    );
    assert_eq!(multi_with_out.code, 2);
    assert!(
        multi_with_out
            .stderr
            .contains("multiple presets does not support --out"),
        "stderr: {}",
        multi_with_out.stderr
    );
}

#[test]
fn generate_duplicate_presets_in_out_dir_are_usage_collision_error() {
    let dir = tempfile::TempDir::new().unwrap();

    let stub = common::make_stub_dir();
    let path_s = stub.path().to_string_lossy().to_string();
    let envs = [("PATH", path_s.as_str())];

    let out = common::run_image_processing(
        dir.path(),
        &[
            "generate",
            "--preset",
            "info",
            "--preset",
            "info",
            "--to",
            "png",
            "--size",
            "32",
            "--out-dir",
            "out",
            "--json",
        ],
        &envs,
    );
    assert_eq!(out.code, 2);
    assert!(
        out.stderr.contains("output collisions detected"),
        "stderr: {}",
        out.stderr
    );
}

#[test]
fn generate_overwrite_flag_controls_existing_output_replacement() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join("out")).unwrap();
    fs::write(dir.path().join("out/info.png"), "existing").unwrap();

    let stub = common::make_stub_dir();
    let path_s = stub.path().to_string_lossy().to_string();
    let envs = [("PATH", path_s.as_str())];

    let blocked = common::run_image_processing(
        dir.path(),
        &[
            "generate",
            "--preset",
            "info",
            "--to",
            "png",
            "--size",
            "32",
            "--out",
            "out/info.png",
            "--json",
        ],
        &envs,
    );
    assert_eq!(blocked.code, 2);
    assert!(
        blocked
            .stderr
            .contains("output exists (pass --overwrite to replace)"),
        "stderr: {}",
        blocked.stderr
    );

    let replaced = common::run_image_processing(
        dir.path(),
        &[
            "generate",
            "--preset",
            "info",
            "--to",
            "png",
            "--size",
            "32",
            "--out",
            "out/info.png",
            "--overwrite",
            "--json",
        ],
        &envs,
    );
    assert_eq!(replaced.code, 0, "stderr: {}", replaced.stderr);
    let rendered = fs::read(dir.path().join("out/info.png")).unwrap();
    assert!(rendered != b"existing");
    let v: serde_json::Value = serde_json::from_str(&replaced.stdout).unwrap();
    assert_eq!(v["items"][0]["status"], "ok");
    assert_eq!(v["items"][0]["output_info"]["format"], "PNG");
}
