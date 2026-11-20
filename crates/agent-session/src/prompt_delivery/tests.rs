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
        composer_image_label(&pane("[Image #2]"), 1, ""),
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
        assert_eq!(composer_image_label(&pane(input), 1, ""), None, "{input}");
    }
    assert_eq!(composer_image_label("unrecognized pane", 1, ""), None);
    assert_eq!(
        composer_image_label(&pane("\u{1b}[2m[Image #2]\u{1b}[22m"), 1, ""),
        None
    );
}

#[test]
fn image_labels_may_be_followed_only_by_the_prompts_remaining_text() {
    let tmp = tempfile::TempDir::new().unwrap();
    for name in ["test image.PNG", "other.png"] {
        fs::write(tmp.path().join(name), b"fixture").unwrap();
    }
    let cwd = tmp.path().to_str().unwrap();
    for (prompt, count, rest) in [
        ("\"test image.PNG\"\n> explain", 1, "> explain"),
        ("'test image.PNG' explain", 1, "explain"),
        ("test image.PNG\n\nexplain\nmore", 1, "explain\nmore"),
        ("test image.PNG", 1, ""),
        ("'test image.PNG'\n'other.png'\n> explain", 2, "> explain"),
        ("test image.PNG\n'other.png'", 2, ""),
        ("'test image.PNG'\n'missing.png'", 1, "'missing.png'"),
        ("'test image.PNG'\n'quoted remark'", 1, "'quoted remark'"),
    ] {
        assert_eq!(
            image_prompt(prompt, cwd),
            Some((count, rest.to_owned())),
            "{prompt:?}"
        );
    }
    for prompt in [
        "explain \"test image.PNG\"",
        "missing.png\nexplain",
        "explain",
        "'other.png\nexplain",
        "'missing.png'\n'other.png'",
    ] {
        assert_eq!(image_prompt(prompt, cwd), None, "{prompt:?}");
    }
    let pane = |input: &str| format!("────────────────────\n❯ {input}\n────────────────────\n");
    for (shown, count) in [
        ("[Image #2] explain it", 1),
        ("[Image #2]explain  it", 1),
        ("[Image #2][Image #3] explain it", 2),
        ("[Image #4] [Image #5]explain it", 2),
    ] {
        assert_eq!(
            composer_image_label(&pane(shown), count, "explain it"),
            Some(shown.into()),
            "{shown}"
        );
    }
    for (shown, count) in [
        ("[Image #2] explain", 1),
        ("[Image #2]", 1),
        ("[Image #2] explain it [Image #3]", 1),
        ("[Image #2][Image #3] explain it", 1),
        ("[Image #2] explain it", 2),
        ("[Image #2][Image #4] explain it", 2),
        ("[Image #3][Image #2] explain it", 2),
        ("[Image #2][Image #3][Image #4] explain it", 2),
    ] {
        assert_eq!(
            composer_image_label(&pane(shown), count, "explain it"),
            None,
            "{shown}"
        );
    }
}

mod startup;
