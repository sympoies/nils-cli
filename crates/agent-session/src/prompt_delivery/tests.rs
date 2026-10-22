use super::*;
use pretty_assertions::assert_eq;

#[test]
fn image_rewrite_requires_a_single_existing_image_path() {
    let tmp = tempfile::TempDir::new().unwrap();
    fs::write(tmp.path().join("test image.PNG"), b"fixture").unwrap();
    fs::write(tmp.path().join("document.txt"), b"fixture").unwrap();
    let cwd = tmp.path().to_str().unwrap();
    for path in ["test image.PNG", "\"test image.PNG\"", "'test image.PNG'"] {
        assert!(image_path_prompt(path, cwd), "{path}");
    }
    for text in [
        "missing.png",
        "document.txt",
        "[Image #2]",
        "\"test image.PNG\" explain",
        "test image.PNG\nexplain",
    ] {
        assert!(!image_path_prompt(text, cwd), "{text}");
    }
}

#[test]
fn image_rewrite_requires_an_exact_observed_composer_label() {
    let pane = |input: &str| format!("────────────────────\n❯ {input}\n────────────────────\n");
    assert_eq!(
        composer_image_label(&pane("[Image #2]")),
        Some("[Image #2]".into())
    );
    for input in [
        "[Image #0]",
        "[Image #]",
        "[Image #x]",
        "[Image #4294967296]",
        "[Image #2] explain",
        "[Image #2] [Image #3]",
    ] {
        assert_eq!(composer_image_label(&pane(input)), None, "{input}");
    }
    assert_eq!(composer_image_label("unrecognized pane"), None);
    assert_eq!(
        composer_image_label(&pane("\u{1b}[2m[Image #2]\u{1b}[22m")),
        None
    );
}
