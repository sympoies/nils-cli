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
fn removed_generate_entrypoint_is_usage_error() {
    let dir = tempfile::TempDir::new().unwrap();

    let out = common::run_image_processing(dir.path(), &["generate"], &[]);
    assert_eq!(out.code, 2);
    assert!(
        out.stderr.contains("invalid value 'generate'")
            || out.stderr.contains("unrecognized subcommand")
            || out.stderr.contains("unknown subcommand"),
        "stderr: {}",
        out.stderr
    );
}

#[test]
fn from_svg_rejects_invalid_input_flags() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::write(dir.path().join("a.png"), "img").unwrap();
    fs::write(
        dir.path().join("icon.svg"),
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32">
<rect x="2" y="2" width="28" height="28" fill="#0f62fe"/>
</svg>"##,
    )
    .unwrap();

    let stub = common::make_stub_dir();
    let path_s = stub.path().to_string_lossy().to_string();
    let envs = [("PATH", path_s.as_str())];

    let with_in = common::run_image_processing(
        dir.path(),
        &[
            "convert",
            "--from-svg",
            "icon.svg",
            "--in",
            "a.png",
            "--to",
            "png",
            "--out",
            "out/icon.png",
            "--json",
        ],
        &envs,
    );
    assert_eq!(with_in.code, 2);
    assert!(
        with_in
            .stderr
            .contains("convert --from-svg does not support --in"),
        "stderr: {}",
        with_in.stderr
    );

    let with_in_place = common::run_image_processing(
        dir.path(),
        &[
            "convert",
            "--from-svg",
            "icon.svg",
            "--in-place",
            "--yes",
            "--to",
            "png",
            "--out",
            "out/icon.png",
            "--json",
        ],
        &envs,
    );
    assert_eq!(with_in_place.code, 2);
    assert!(
        with_in_place
            .stderr
            .contains("convert --from-svg does not support --in-place"),
        "stderr: {}",
        with_in_place.stderr
    );
}

#[test]
fn from_svg_rejects_output_mode_mismatches() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::write(
        dir.path().join("icon.svg"),
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32">
<rect x="2" y="2" width="28" height="28" fill="#0f62fe"/>
</svg>"##,
    )
    .unwrap();

    let stub = common::make_stub_dir();
    let path_s = stub.path().to_string_lossy().to_string();
    let envs = [("PATH", path_s.as_str())];

    let with_out_dir = common::run_image_processing(
        dir.path(),
        &[
            "convert",
            "--from-svg",
            "icon.svg",
            "--to",
            "png",
            "--out-dir",
            "out",
            "--json",
        ],
        &envs,
    );
    assert_eq!(with_out_dir.code, 2);
    assert!(
        with_out_dir
            .stderr
            .contains("convert --from-svg requires --out"),
        "stderr: {}",
        with_out_dir.stderr
    );

    let invalid_to = common::run_image_processing(
        dir.path(),
        &[
            "convert",
            "--from-svg",
            "icon.svg",
            "--to",
            "jpg",
            "--out",
            "out/icon.jpg",
            "--json",
        ],
        &envs,
    );
    assert_eq!(invalid_to.code, 2);
    assert!(
        invalid_to.stderr.contains("png|webp|svg"),
        "stderr: {}",
        invalid_to.stderr
    );
}

#[test]
fn svg_validate_invalid_svg_returns_actionable_error() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::write(
        dir.path().join("invalid.svg"),
        r##"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"##,
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
            "invalid.svg",
            "--out",
            "out/invalid.cleaned.svg",
            "--json",
        ],
        &envs,
    );
    assert_eq!(out.code, 1, "stderr: {}", out.stderr);
    let v: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(v["items"][0]["status"], "error");
    let error = v["items"][0]["error"].as_str().unwrap_or("");
    assert!(
        error.contains("missing_viewbox")
            || error.contains("disallowed_tag")
            || error.contains("unsafe_tag"),
        "error: {error}"
    );
}

#[test]
fn from_svg_overwrite_flag_controls_existing_output_replacement() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::write(
        dir.path().join("icon.svg"),
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32">
<rect x="3" y="3" width="26" height="26" rx="3" fill="#0f62fe"/>
</svg>"##,
    )
    .unwrap();
    fs::create_dir_all(dir.path().join("out")).unwrap();
    fs::write(dir.path().join("out/icon.png"), "existing").unwrap();

    let stub = common::make_stub_dir();
    let path_s = stub.path().to_string_lossy().to_string();
    let envs = [("PATH", path_s.as_str())];

    let blocked = common::run_image_processing(
        dir.path(),
        &[
            "convert",
            "--from-svg",
            "icon.svg",
            "--to",
            "png",
            "--out",
            "out/icon.png",
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
            "convert",
            "--from-svg",
            "icon.svg",
            "--to",
            "png",
            "--out",
            "out/icon.png",
            "--overwrite",
            "--json",
        ],
        &envs,
    );
    assert_eq!(replaced.code, 0, "stderr: {}", replaced.stderr);
    let rendered = fs::read(dir.path().join("out/icon.png")).unwrap();
    assert!(rendered != b"existing");
    let v: serde_json::Value = serde_json::from_str(&replaced.stdout).unwrap();
    assert_eq!(v["items"][0]["status"], "ok");
    assert_eq!(v["items"][0]["output_info"]["format"], "PNG");
}
