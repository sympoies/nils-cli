//! De-identified post-paste layouts and explicitly synthetic empty controls.
use super::exact_composer_input;
use pretty_assertions::assert_eq;

#[test]
fn captured_statuslines_preserve_drafts_and_empty_controls() {
    let data: serde_json::Value =
        serde_json::from_str(include_str!("claude-statusline-captures.json")).unwrap();
    for case in data["fixtures"].as_array().unwrap() {
        assert_eq!(
            exact_composer_input("claude", case["capture"].as_str().unwrap()),
            (case["expected_observation"] == "empty").then(String::new),
            "extra logical rows refuse; a fully dim single-row hint stays empty"
        );
        let template = case["template"].as_str().unwrap();
        for (input, expected) in [
            ("", ""),
            ("\u{1b}[2mContinue the fixture check\u{1b}[0m", ""),
            ("user draft ", "user draft "),
            ("\u{1b}[38;5;246muser draft\u{1b}[39m", "user draft"),
        ] {
            assert_eq!(
                exact_composer_input("claude", &template.replace("{{input}}", input)).as_deref(),
                Some(expected),
                "case {}",
                case["case"]
            );
        }
    }
}
