#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    pub id: u32,
    pub owner_name: String,
    pub title: String,
    pub bounds: Rect,
    pub on_screen: bool,
    pub owner_pid: i32,
    pub z_order: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppInfo {
    pub name: String,
    pub pid: i32,
    pub bundle_id: String,
}

#[derive(Debug, Clone, Default)]
pub struct ShareableContent {
    pub windows: Vec<WindowInfo>,
    pub apps: Vec<AppInfo>,
}
