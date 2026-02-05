use screen_record::select::{select_window, SelectionArgs};
use screen_record::types::{Rect, WindowInfo};

fn window(
    id: u32,
    owner: &str,
    title: &str,
    on_screen: bool,
    active: bool,
    z: usize,
) -> WindowInfo {
    WindowInfo {
        id,
        owner_name: owner.to_string(),
        title: title.to_string(),
        bounds: Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        },
        on_screen,
        active,
        owner_pid: 1,
        z_order: z,
    }
}

#[test]
fn ambiguous_app_selection_outputs_candidates() {
    let windows = vec![
        window(10, "Terminal", "Inbox", false, false, 0),
        window(11, "Terminal", "Docs", false, false, 1),
    ];
    let args = SelectionArgs {
        app: Some("Terminal".to_string()),
        ..SelectionArgs::default()
    };

    let err = select_window(&windows, &args).expect_err("ambiguous error");
    let message = err.to_string();
    assert!(message.contains("multiple windows match --app \"Terminal\""));
    assert!(message.contains("Terminal\tDocs"));
    assert!(message.contains("Terminal\tInbox"));
    assert_eq!(err.exit_code(), 2);
}
