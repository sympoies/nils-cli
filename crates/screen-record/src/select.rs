use crate::error::CliError;
use crate::types::WindowInfo;

#[derive(Debug, Clone, Default)]
pub struct SelectionArgs {
    pub window_id: Option<u32>,
    pub app: Option<String>,
    pub window_name: Option<String>,
    pub active_window: bool,
}

pub fn select_window(windows: &[WindowInfo], args: &SelectionArgs) -> Result<WindowInfo, CliError> {
    if let Some(id) = args.window_id {
        return windows
            .iter()
            .find(|window| window.id == id)
            .cloned()
            .ok_or_else(|| CliError::usage(format!("no window found with id {id}")));
    }

    if args.active_window {
        return select_frontmost(windows).ok_or_else(|| CliError::usage("no active window found"));
    }

    let Some(app) = args.app.as_deref() else {
        return Err(CliError::usage("missing selection flag"));
    };

    let mut candidates: Vec<WindowInfo> = windows
        .iter()
        .filter(|window| contains_case_insensitive(&window.owner_name, app))
        .cloned()
        .collect();

    if let Some(window_name) = args.window_name.as_deref() {
        candidates.retain(|window| contains_case_insensitive(&window.title, window_name));
    }

    if candidates.is_empty() {
        return Err(CliError::usage(format!("no windows match --app \"{app}\"")));
    }

    if args.window_name.is_some() {
        if candidates.len() == 1 {
            return Ok(candidates.remove(0));
        }
        return Err(ambiguous_app_error(app, &candidates));
    }

    let frontmost = frontmost_for_app(&candidates);
    match frontmost {
        Some(window) => Ok(window),
        None => Err(ambiguous_app_error(app, &candidates)),
    }
}

fn select_frontmost(windows: &[WindowInfo]) -> Option<WindowInfo> {
    windows
        .iter()
        .filter(|window| window.active && window.on_screen)
        .min_by_key(|window| window.z_order)
        .cloned()
        .or_else(|| {
            windows
                .iter()
                .filter(|window| window.on_screen)
                .min_by_key(|window| window.z_order)
                .cloned()
        })
        .or_else(|| {
            windows
                .iter()
                .filter(|window| window.active)
                .min_by_key(|window| window.z_order)
                .cloned()
        })
}

fn frontmost_for_app(candidates: &[WindowInfo]) -> Option<WindowInfo> {
    if let Some(window) = pick_unique_by_z_order(
        candidates
            .iter()
            .filter(|window| window.active && window.on_screen),
    ) {
        return Some(window);
    }

    if let Some(window) =
        pick_unique_by_z_order(candidates.iter().filter(|window| window.on_screen))
    {
        return Some(window);
    }

    pick_unique_by_z_order(candidates.iter().filter(|window| window.active))
}

fn contains_case_insensitive(haystack: &str, needle: &str) -> bool {
    haystack
        .to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

fn ambiguous_app_error(app: &str, candidates: &[WindowInfo]) -> CliError {
    let mut sorted = candidates.to_vec();
    sorted.sort_by(|a, b| {
        a.owner_name
            .cmp(&b.owner_name)
            .then_with(|| a.title.cmp(&b.title))
            .then_with(|| a.id.cmp(&b.id))
    });

    let mut message = format!(
        "error: multiple windows match --app \"{app}\"\nerror: refine with --window-name or use --window-id"
    );

    for window in sorted {
        message.push('\n');
        message.push_str(&format_window_tsv(&window));
    }

    CliError::usage(message)
}

pub fn format_window_tsv(window: &WindowInfo) -> String {
    format!(
        "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
        window.id,
        normalize_tsv_field(&window.owner_name),
        normalize_tsv_field(&window.title),
        window.bounds.x,
        window.bounds.y,
        window.bounds.width,
        window.bounds.height,
        if window.on_screen { "true" } else { "false" }
    )
}

fn pick_unique_by_z_order<'a, I>(iter: I) -> Option<WindowInfo>
where
    I: IntoIterator<Item = &'a WindowInfo>,
{
    let mut list: Vec<&WindowInfo> = iter.into_iter().collect();
    if list.is_empty() {
        return None;
    }
    list.sort_by_key(|window| window.z_order);
    let best = list[0];
    if list
        .iter()
        .skip(1)
        .any(|window| window.z_order == best.z_order)
    {
        return None;
    }
    Some(best.clone())
}

fn normalize_tsv_field(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch == '\t' || ch == '\n' || ch == '\r' {
                ' '
            } else {
                ch
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Rect;

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
    fn select_by_window_id() {
        let windows = vec![window(10, "Terminal", "Inbox", true, false, 0)];
        let args = SelectionArgs {
            window_id: Some(10),
            ..SelectionArgs::default()
        };
        let selected = select_window(&windows, &args).expect("select window");
        assert_eq!(selected.id, 10);
    }

    #[test]
    fn select_by_app_picks_frontmost() {
        let windows = vec![
            window(10, "Terminal", "Inbox", true, false, 1),
            window(11, "Terminal", "Docs", true, false, 0),
        ];
        let args = SelectionArgs {
            app: Some("Terminal".to_string()),
            ..SelectionArgs::default()
        };
        let selected = select_window(&windows, &args).expect("select window");
        assert_eq!(selected.id, 11);
    }

    #[test]
    fn select_by_app_and_window_name() {
        let windows = vec![
            window(10, "Terminal", "Inbox", true, false, 0),
            window(11, "Terminal", "Docs", true, false, 1),
        ];
        let args = SelectionArgs {
            app: Some("Terminal".to_string()),
            window_name: Some("Docs".to_string()),
            ..SelectionArgs::default()
        };
        let selected = select_window(&windows, &args).expect("select window");
        assert_eq!(selected.id, 11);
    }

    #[test]
    fn ambiguous_app_selection_errors() {
        let windows = vec![
            window(10, "Terminal", "Inbox", false, false, 0),
            window(11, "Terminal", "Docs", false, false, 1),
        ];
        let args = SelectionArgs {
            app: Some("Terminal".to_string()),
            ..SelectionArgs::default()
        };
        let err = select_window(&windows, &args).expect_err("ambiguous error");
        assert_eq!(err.exit_code(), 2);
        assert!(
            err.to_string()
                .contains("multiple windows match --app \"Terminal\"")
        );
    }

    #[test]
    fn select_active_window_prefers_active() {
        let windows = vec![
            window(10, "Terminal", "Inbox", true, false, 0),
            window(11, "Terminal", "Docs", true, true, 5),
        ];
        let args = SelectionArgs {
            active_window: true,
            ..SelectionArgs::default()
        };
        let selected = select_window(&windows, &args).expect("select window");
        assert_eq!(selected.id, 11);
    }

    #[test]
    fn select_by_app_prefers_active() {
        let windows = vec![
            window(10, "Terminal", "Inbox", true, false, 0),
            window(11, "Terminal", "Docs", true, true, 3),
        ];
        let args = SelectionArgs {
            app: Some("Terminal".to_string()),
            ..SelectionArgs::default()
        };
        let selected = select_window(&windows, &args).expect("select window");
        assert_eq!(selected.id, 11);
    }
}
