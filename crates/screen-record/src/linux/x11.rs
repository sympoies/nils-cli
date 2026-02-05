use std::collections::BTreeMap;

use x11rb::connection::Connection;
use x11rb::protocol::randr::{self, ConnectionExt as RandrExt};
use x11rb::protocol::xproto::{self, Atom, AtomEnum, ConnectionExt as XprotoExt, Window};

use crate::error::CliError;
use crate::types::{AppInfo, DisplayInfo, Rect, ShareableContent, WindowInfo};

pub fn fetch_shareable_content() -> Result<ShareableContent, CliError> {
    let (conn, screen_num) = x11rb::connect(None)
        .map_err(|err| CliError::runtime(format!("failed to connect to X11: {err}")))?;
    let setup = conn.setup();
    let screen = setup
        .roots
        .get(screen_num)
        .ok_or_else(|| CliError::runtime("failed to resolve X11 screen"))?;
    let root = screen.root;

    let atoms = Atoms::new(&conn)?;
    let active_window = get_window_list(
        &conn,
        root,
        atoms.net_active_window,
        AtomEnum::WINDOW.into(),
    )?
    .and_then(|list| list.first().copied());

    let client_list = get_window_list(&conn, root, atoms.net_client_list, AtomEnum::WINDOW.into())?;
    let stacking_list = get_window_list(
        &conn,
        root,
        atoms.net_client_list_stacking,
        AtomEnum::WINDOW.into(),
    )?;

    let mut candidates = if let Some(list) = client_list {
        list
    } else {
        query_tree_windows(&conn, root)?
    };

    let fallback_z = z_order_map(&candidates);
    let z_order_map = match stacking_list.as_ref() {
        Some(list) if !list.is_empty() => {
            let mut map = z_order_map(list);
            for (id, order) in &fallback_z {
                map.entry(*id).or_insert(*order);
            }
            map
        }
        _ => fallback_z,
    };

    let mut windows = Vec::new();
    for window in candidates.drain(..) {
        let geom = match conn.get_geometry(window) {
            Ok(cookie) => match cookie.reply() {
                Ok(reply) => reply,
                Err(_) => continue,
            },
            Err(_) => continue,
        };
        let translate = match conn.translate_coordinates(window, root, 0, 0) {
            Ok(cookie) => cookie.reply().ok(),
            Err(_) => None,
        };

        let attrs = match conn.get_window_attributes(window) {
            Ok(cookie) => cookie.reply().ok(),
            Err(_) => None,
        };

        let map_state = attrs.as_ref().map(|attr| attr.map_state);
        let mut on_screen = matches!(map_state, Some(xproto::MapState::VIEWABLE));

        if on_screen
            && window_has_state(&conn, window, atoms.net_wm_state, atoms.net_wm_state_hidden)
        {
            on_screen = false;
        }

        let title = window_title(&conn, window, &atoms);
        let owner_name = window_owner_name(&conn, window, &atoms);
        let owner_pid = window_owner_pid(&conn, window, atoms.net_wm_pid);

        let bounds = Rect {
            x: translate
                .map(|reply| reply.dst_x as i32)
                .unwrap_or(geom.x as i32),
            y: translate
                .map(|reply| reply.dst_y as i32)
                .unwrap_or(geom.y as i32),
            width: geom.width as i32,
            height: geom.height as i32,
        };

        windows.push(WindowInfo {
            id: window,
            owner_name,
            title,
            bounds,
            on_screen,
            active: Some(window) == active_window,
            owner_pid,
            z_order: z_order_map.get(&window).copied().unwrap_or(usize::MAX),
        });
    }

    let displays = query_displays(&conn, screen, root)?;
    let apps = derive_apps(&windows);

    Ok(ShareableContent {
        displays,
        windows,
        apps,
    })
}

fn query_displays<C: Connection>(
    conn: &C,
    screen: &xproto::Screen,
    root: Window,
) -> Result<Vec<DisplayInfo>, CliError> {
    let resources = match conn.randr_get_screen_resources_current(root) {
        Ok(cookie) => match cookie.reply() {
            Ok(reply) => reply,
            Err(_) => return Ok(fallback_display(screen)),
        },
        Err(_) => return Ok(fallback_display(screen)),
    };

    let mut displays = Vec::new();
    for output in resources.outputs {
        let info = match conn.randr_get_output_info(output, resources.config_timestamp) {
            Ok(cookie) => match cookie.reply() {
                Ok(reply) => reply,
                Err(_) => continue,
            },
            Err(_) => continue,
        };

        if info.connection != randr::Connection::CONNECTED {
            continue;
        }

        if info.crtc == 0 {
            continue;
        }

        let crtc_info = match conn.randr_get_crtc_info(info.crtc, resources.config_timestamp) {
            Ok(cookie) => match cookie.reply() {
                Ok(reply) => reply,
                Err(_) => continue,
            },
            Err(_) => continue,
        };

        if crtc_info.width == 0 || crtc_info.height == 0 {
            continue;
        }

        displays.push(DisplayInfo {
            id: output,
            width: crtc_info.width as i32,
            height: crtc_info.height as i32,
        });
    }

    if displays.is_empty() {
        Ok(fallback_display(screen))
    } else {
        Ok(displays)
    }
}

fn fallback_display(screen: &xproto::Screen) -> Vec<DisplayInfo> {
    vec![DisplayInfo {
        id: 1,
        width: screen.width_in_pixels as i32,
        height: screen.height_in_pixels as i32,
    }]
}

fn query_tree_windows<C: Connection>(conn: &C, root: Window) -> Result<Vec<Window>, CliError> {
    let reply = match conn.query_tree(root) {
        Ok(cookie) => cookie
            .reply()
            .map_err(|err| CliError::runtime(format!("failed to query X11 window tree: {err}")))?,
        Err(err) => {
            return Err(CliError::runtime(format!(
                "failed to query X11 window tree: {err}"
            )))
        }
    };

    let mut windows = Vec::new();
    for window in reply.children {
        let attrs = match conn.get_window_attributes(window) {
            Ok(cookie) => match cookie.reply() {
                Ok(reply) => reply,
                Err(_) => continue,
            },
            Err(_) => continue,
        };

        if attrs.map_state == xproto::MapState::VIEWABLE {
            windows.push(window);
        }
    }
    Ok(windows)
}

fn get_window_list<C: Connection>(
    conn: &C,
    window: Window,
    property: Atom,
    property_type: Atom,
) -> Result<Option<Vec<Window>>, CliError> {
    let reply = match conn.get_property(false, window, property, property_type, 0, u32::MAX) {
        Ok(cookie) => match cookie.reply() {
            Ok(reply) => reply,
            Err(_) => return Ok(None),
        },
        Err(_) => return Ok(None),
    };

    let list = reply
        .value32()
        .map(|iter| iter.map(|value| value as Window).collect::<Vec<_>>());

    Ok(list.filter(|values| !values.is_empty()))
}

fn z_order_map(windows: &[Window]) -> std::collections::HashMap<Window, usize> {
    let total = windows.len();
    windows
        .iter()
        .enumerate()
        .map(|(idx, window)| (*window, total.saturating_sub(idx + 1)))
        .collect()
}

fn window_title<C: Connection>(conn: &C, window: Window, atoms: &Atoms) -> String {
    if let Some(bytes) = get_property_bytes(conn, window, atoms.net_wm_name, atoms.utf8_string) {
        if let Some(title) = bytes_to_string(&bytes) {
            return title;
        }
    }

    if let Some(bytes) = get_property_bytes(conn, window, atoms.wm_name, AtomEnum::STRING.into()) {
        if let Some(title) = bytes_to_string(&bytes) {
            return title;
        }
    }

    String::new()
}

fn window_owner_name<C: Connection>(conn: &C, window: Window, atoms: &Atoms) -> String {
    let bytes = get_property_bytes(conn, window, atoms.wm_class, AtomEnum::STRING.into());
    if let Some(bytes) = bytes {
        let parts: Vec<String> = bytes
            .split(|ch| *ch == 0)
            .filter(|part| !part.is_empty())
            .map(|part| String::from_utf8_lossy(part).trim().to_string())
            .filter(|part| !part.is_empty())
            .collect();
        if let Some(class) = parts.get(1).or_else(|| parts.first()) {
            return class.to_string();
        }
    }

    "Unknown".to_string()
}

fn window_owner_pid<C: Connection>(conn: &C, window: Window, property: Atom) -> i32 {
    let reply = match conn.get_property(false, window, property, AtomEnum::CARDINAL, 0, 1) {
        Ok(cookie) => match cookie.reply() {
            Ok(reply) => reply,
            Err(_) => return 0,
        },
        Err(_) => return 0,
    };
    reply
        .value32()
        .and_then(|mut iter| iter.next())
        .map(|value| value as i32)
        .unwrap_or(0)
}

fn window_has_state<C: Connection>(
    conn: &C,
    window: Window,
    property: Atom,
    hidden_state: Atom,
) -> bool {
    let reply = match conn.get_property(false, window, property, AtomEnum::ATOM, 0, u32::MAX) {
        Ok(cookie) => match cookie.reply() {
            Ok(reply) => reply,
            Err(_) => return false,
        },
        Err(_) => return false,
    };

    // Avoid keeping the borrow from `reply.value32()` alive until the end of the block (E0597).
    let has_state = match reply.value32() {
        Some(mut iter) => iter.any(|value| value == hidden_state),
        None => false,
    };
    has_state
}

fn get_property_bytes<C: Connection>(
    conn: &C,
    window: Window,
    property: Atom,
    property_type: Atom,
) -> Option<Vec<u8>> {
    let cookie = conn
        .get_property(false, window, property, property_type, 0, u32::MAX)
        .ok()?;
    let reply = cookie.reply().ok()?;
    if reply.value_len == 0 {
        return None;
    }
    Some(reply.value)
}

fn bytes_to_string(bytes: &[u8]) -> Option<String> {
    let value = String::from_utf8_lossy(bytes).trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn derive_apps(windows: &[WindowInfo]) -> Vec<AppInfo> {
    let mut apps = BTreeMap::new();
    for window in windows {
        let key = (window.owner_name.clone(), window.owner_pid);
        apps.entry(key).or_insert_with(|| AppInfo {
            name: window.owner_name.clone(),
            pid: window.owner_pid,
            bundle_id: String::new(),
        });
    }
    apps.into_values().collect()
}

struct Atoms {
    net_client_list: Atom,
    net_client_list_stacking: Atom,
    net_active_window: Atom,
    net_wm_name: Atom,
    utf8_string: Atom,
    wm_name: Atom,
    wm_class: Atom,
    net_wm_pid: Atom,
    net_wm_state: Atom,
    net_wm_state_hidden: Atom,
}

impl Atoms {
    fn new<C: Connection>(conn: &C) -> Result<Self, CliError> {
        Ok(Self {
            net_client_list: intern_atom(conn, "_NET_CLIENT_LIST")?,
            net_client_list_stacking: intern_atom(conn, "_NET_CLIENT_LIST_STACKING")?,
            net_active_window: intern_atom(conn, "_NET_ACTIVE_WINDOW")?,
            net_wm_name: intern_atom(conn, "_NET_WM_NAME")?,
            utf8_string: intern_atom(conn, "UTF8_STRING")?,
            wm_name: intern_atom(conn, "WM_NAME")?,
            wm_class: intern_atom(conn, "WM_CLASS")?,
            net_wm_pid: intern_atom(conn, "_NET_WM_PID")?,
            net_wm_state: intern_atom(conn, "_NET_WM_STATE")?,
            net_wm_state_hidden: intern_atom(conn, "_NET_WM_STATE_HIDDEN")?,
        })
    }
}

fn intern_atom<C: Connection>(conn: &C, name: &str) -> Result<Atom, CliError> {
    let cookie = conn
        .intern_atom(false, name.as_bytes())
        .map_err(|err| CliError::runtime(format!("failed to intern atom {name}: {err}")))?;
    let reply = cookie
        .reply()
        .map_err(|err| CliError::runtime(format!("failed to intern atom {name}: {err}")))?;
    Ok(reply.atom)
}
