fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "macos" {
        return;
    }

    println!("cargo:rerun-if-env-changed=CARGO_CFG_TARGET_OS");

    let frameworks = [
        "ScreenCaptureKit",
        "AVFoundation",
        "CoreMedia",
        "CoreVideo",
        "CoreGraphics",
        "ImageIO",
        "Foundation",
    ];

    for framework in frameworks {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
}
