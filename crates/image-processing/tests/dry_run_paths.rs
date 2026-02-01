mod common;

use std::fs;

use pretty_assertions::assert_eq;

#[test]
fn convert_alpha_jpg_dry_run_includes_background_quality_strip_auto_orient() {
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
            "--background",
            "#fff",
            "--quality",
            "80",
            "--strip-metadata",
            "--dry-run",
            "--json",
        ],
        &envs,
    );
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);

    let v: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    let joined = v["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("-auto-orient"), "cmds: {joined}");
    assert!(joined.contains("-background '#fff'"), "cmds: {joined}");
    assert!(joined.contains("-alpha remove"), "cmds: {joined}");
    assert!(joined.contains("-alpha off"), "cmds: {joined}");
    assert!(joined.contains("-quality 80"), "cmds: {joined}");
    assert!(joined.contains("-strip"), "cmds: {joined}");

    assert!(
        !dir.path().join("out/a.jpg").exists(),
        "dry-run should not write output"
    );
}

#[test]
fn resize_dry_run_cover_no_pre_upscale_omits_pre_upscale_step() {
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
            "--width",
            "120",
            "--height",
            "60",
            "--fit",
            "cover",
            "--no-pre-upscale",
            "--out",
            "out/a.png",
            "--dry-run",
            "--json",
        ],
        &envs,
    );
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);

    let v: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    let joined = v["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!joined.contains("200%"), "cmds: {joined}");
    assert!(joined.contains("120x60^"), "cmds: {joined}");
    assert!(joined.contains("-extent 120x60"), "cmds: {joined}");
}

#[test]
fn pad_dry_run_with_background_includes_extent() {
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
            "--background",
            "#000",
            "--out",
            "out/a.jpg",
            "--dry-run",
            "--json",
        ],
        &envs,
    );
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);

    let v: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    let joined = v["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("-background '#000'"), "cmds: {joined}");
    assert!(joined.contains("-extent 120x60"), "cmds: {joined}");
}

#[test]
fn rotate_dry_run_non_right_angle_adds_background_and_strip() {
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
            "45",
            "--out",
            "out/a.png",
            "--strip-metadata",
            "--dry-run",
            "--json",
        ],
        &envs,
    );
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);

    let v: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    let joined = v["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("-background none"), "cmds: {joined}");
    assert!(joined.contains("-rotate 45"), "cmds: {joined}");
    assert!(joined.contains("-strip"), "cmds: {joined}");
}

#[test]
fn crop_rect_dry_run_includes_rect_geometry() {
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
            "crop",
            "--in",
            "a.png",
            "--rect",
            "10x5+2+3",
            "--out",
            "out/a.png",
            "--dry-run",
            "--json",
        ],
        &envs,
    );
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);

    let v: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    let joined = v["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("-crop 10x5+2+3"), "cmds: {joined}");
    assert!(!joined.contains("-gravity"), "cmds: {joined}");
}

#[test]
fn crop_size_dry_run_uses_gravity_and_crop() {
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
            "crop",
            "--in",
            "a.png",
            "--size",
            "20x10",
            "--gravity",
            "north",
            "--out",
            "out/a.png",
            "--dry-run",
            "--json",
        ],
        &envs,
    );
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);

    let v: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    let joined = v["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("-gravity north"), "cmds: {joined}");
    assert!(joined.contains("-crop 20x10+0+0"), "cmds: {joined}");
}

#[test]
fn flip_and_flop_dry_run_include_flags() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::write(dir.path().join("a.png"), "img").unwrap();

    let stub = common::make_stub_dir();
    common::write_exe(stub.path(), "identify", common::identify_stub_script());
    common::write_exe(stub.path(), "convert", common::convert_stub_script());

    let path_s = stub.path().to_string_lossy().to_string();
    let envs = [("PATH", path_s.as_str())];

    for (op, flag) in [("flip", "-flip"), ("flop", "-flop")] {
        let out = common::run_image_processing(
            dir.path(),
            &[
                op,
                "--in",
                "a.png",
                "--out",
                "out/a.png",
                "--dry-run",
                "--json",
            ],
            &envs,
        );
        assert_eq!(out.code, 0, "stderr: {}", out.stderr);
        let v: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
        let joined = v["commands"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains(flag), "cmds: {joined}");
    }
}

#[test]
fn optimize_jpg_dry_run_falls_back_to_magick() {
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
            "optimize",
            "--in",
            "a.jpg",
            "--out",
            "out/a.jpg",
            "--dry-run",
            "--json",
        ],
        &envs,
    );
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);

    let v: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    let joined = v["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("-quality 85"), "cmds: {joined}");
    assert!(joined.contains("-interlace Plane"), "cmds: {joined}");
}

#[test]
fn optimize_webp_dry_run_falls_back_to_magick_lossless() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::write(dir.path().join("a.webp"), "img").unwrap();

    let stub = common::make_stub_dir();
    common::write_exe(stub.path(), "identify", common::identify_stub_script());
    common::write_exe(stub.path(), "convert", common::convert_stub_script());

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
            "--lossless",
            "--dry-run",
            "--json",
        ],
        &envs,
    );
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);

    let v: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    let joined = v["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("webp:lossless=true"), "cmds: {joined}");
}
