use std::collections::HashMap;
use std::env;

use crate::error::CliError;
use crate::test_mode;

const ENV_FORCE_AVAILABLE: &str = "CODEX_SCREEN_RECORD_PORTAL_FORCE_AVAILABLE";
const ENV_FORCE_MISSING: &str = "CODEX_SCREEN_RECORD_PORTAL_FORCE_MISSING";

pub const TEST_PIPEWIRE_NODE_ID: u32 = 4242;

const PORTAL_BUS_NAME: &str = "org.freedesktop.portal.Desktop";
const PORTAL_OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";
const PORTAL_SCREENCAST_IFACE: &str = "org.freedesktop.portal.ScreenCast";
const PORTAL_REQUEST_IFACE: &str = "org.freedesktop.portal.Request";

#[derive(Debug)]
pub struct PortalCapture {
    pub node_id: u32,
    pub pipewire_remote: Option<zbus::zvariant::OwnedFd>,
}

fn ov_str(value: &str) -> zbus::zvariant::OwnedValue {
    zbus::zvariant::OwnedValue::from(zbus::zvariant::Str::from(value))
}

pub fn ensure_portal_available() -> Result<(), CliError> {
    if env_flag_enabled(ENV_FORCE_AVAILABLE) {
        return Ok(());
    }
    if env_flag_enabled(ENV_FORCE_MISSING) {
        return Err(portal_missing_error());
    }

    let Ok(connection) = zbus::blocking::Connection::session() else {
        return Err(CliError::runtime(
            "Wayland-only session detected but no DBus session bus is available (DBUS_SESSION_BUS_ADDRESS is unset).",
        ));
    };

    let proxy = zbus::blocking::Proxy::new(
        &connection,
        PORTAL_BUS_NAME,
        PORTAL_OBJECT_PATH,
        PORTAL_SCREENCAST_IFACE,
    )
    .map_err(|err| CliError::runtime(format!("failed to create portal proxy: {err}")))?;

    let version_result: Result<u32, _> = proxy.get_property("version");
    if let Err(err) = version_result {
        let text = err.to_string();
        if text.contains("ServiceUnknown")
            || text.contains("NameHasNoOwner")
            || text.contains("org.freedesktop.portal.Desktop")
        {
            return Err(portal_missing_error());
        }
        return Err(CliError::runtime(format!(
            "failed to query xdg-desktop-portal ScreenCast interface: {err}"
        )));
    }

    Ok(())
}

pub fn acquire_capture() -> Result<PortalCapture, CliError> {
    if test_mode::enabled() {
        return Ok(PortalCapture {
            node_id: TEST_PIPEWIRE_NODE_ID,
            pipewire_remote: None,
        });
    }

    ensure_portal_available()?;

    let connection = zbus::blocking::Connection::session().map_err(|err| {
        CliError::runtime(format!(
            "failed to connect to DBus session bus for portal capture: {err}"
        ))
    })?;

    let proxy = zbus::blocking::Proxy::new(
        &connection,
        PORTAL_BUS_NAME,
        PORTAL_OBJECT_PATH,
        PORTAL_SCREENCAST_IFACE,
    )
    .map_err(|err| CliError::runtime(format!("failed to create portal proxy: {err}")))?;

    let session_handle = create_session(&connection, &proxy)?;
    select_sources(&connection, &proxy, &session_handle)?;
    let node_id = start_session(&connection, &proxy, &session_handle)?;

    let pipewire_remote = proxy
        .call::<_, _, zbus::zvariant::OwnedFd>(
            "OpenPipeWireRemote",
            &(
                session_handle.clone(),
                portal_options([("handle_token", ov_str(&token("pipewire")))]),
            ),
        )
        .map_err(|err| {
            CliError::runtime(format!("failed to open PipeWire remote via portal: {err}"))
        })?;

    Ok(PortalCapture {
        node_id,
        pipewire_remote: Some(pipewire_remote),
    })
}

pub fn acquire_pipewire_node_id() -> Result<u32, CliError> {
    Ok(acquire_capture()?.node_id)
}

fn create_session(
    connection: &zbus::blocking::Connection,
    proxy: &zbus::blocking::Proxy,
) -> Result<zbus::zvariant::OwnedObjectPath, CliError> {
    let options = portal_options([("session_handle_token", ov_str(&token("session")))]);
    let request = proxy
        .call::<_, _, zbus::zvariant::OwnedObjectPath>("CreateSession", &(options,))
        .map_err(|err| CliError::runtime(format!("portal CreateSession failed: {err}")))?;

    let results = wait_request_response(connection, request)?;
    dict_get_objpath(&results, "session_handle").map_err(|err| {
        CliError::runtime(format!(
            "portal CreateSession response missing session_handle: {err}"
        ))
    })
}

fn select_sources(
    connection: &zbus::blocking::Connection,
    proxy: &zbus::blocking::Proxy,
    session: &zbus::zvariant::OwnedObjectPath,
) -> Result<(), CliError> {
    let options = portal_options([
        ("types", zbus::zvariant::OwnedValue::from(1u32 | 2u32)),
        ("multiple", zbus::zvariant::OwnedValue::from(false)),
        ("cursor_mode", zbus::zvariant::OwnedValue::from(2u32)),
        ("handle_token", ov_str(&token("select"))),
    ]);

    let request = proxy
        .call::<_, _, zbus::zvariant::OwnedObjectPath>("SelectSources", &(session.clone(), options))
        .map_err(|err| CliError::runtime(format!("portal SelectSources failed: {err}")))?;

    let _ = wait_request_response(connection, request)?;
    Ok(())
}

fn start_session(
    connection: &zbus::blocking::Connection,
    proxy: &zbus::blocking::Proxy,
    session: &zbus::zvariant::OwnedObjectPath,
) -> Result<u32, CliError> {
    let options = portal_options([("handle_token", ov_str(&token("start")))]);
    let request = proxy
        .call::<_, _, zbus::zvariant::OwnedObjectPath>(
            "Start",
            &(session.clone(), String::new(), options),
        )
        .map_err(|err| CliError::runtime(format!("portal Start failed: {err}")))?;

    let results = wait_request_response(connection, request)?;
    let streams = dict_get_streams(&results, "streams").map_err(|err| {
        CliError::runtime(format!("portal Start response missing streams: {err}"))
    })?;
    let (node_id, _) = streams
        .into_iter()
        .next()
        .ok_or_else(|| CliError::runtime("portal Start returned no streams"))?;
    Ok(node_id)
}

fn wait_request_response(
    connection: &zbus::blocking::Connection,
    request_path: zbus::zvariant::OwnedObjectPath,
) -> Result<HashMap<String, zbus::zvariant::OwnedValue>, CliError> {
    let request = zbus::blocking::Proxy::new(
        connection,
        PORTAL_BUS_NAME,
        request_path,
        PORTAL_REQUEST_IFACE,
    )
    .map_err(|err| CliError::runtime(format!("failed to create portal request proxy: {err}")))?;

    let mut stream = request
        .receive_signal("Response")
        .map_err(|err| CliError::runtime(format!("failed to wait for portal response: {err}")))?;

    let msg = stream
        .next()
        .ok_or_else(|| CliError::runtime("portal response stream ended unexpectedly"))?;

    let (code, results): (u32, HashMap<String, zbus::zvariant::OwnedValue>) = msg
        .body()
        .deserialize()
        .map_err(|err| CliError::runtime(format!("failed to decode portal response: {err}")))?;

    if code != 0 {
        return Err(CliError::runtime(format!(
            "portal request failed or was cancelled (response={code})"
        )));
    }

    Ok(results)
}

fn dict_get_objpath(
    dict: &HashMap<String, zbus::zvariant::OwnedValue>,
    key: &str,
) -> Result<zbus::zvariant::OwnedObjectPath, String> {
    let value = dict.get(key).ok_or_else(|| format!("missing key {key}"))?;
    let owned = value
        .try_clone()
        .map_err(|_| format!("failed to clone key {key}"))?;
    zbus::zvariant::OwnedObjectPath::try_from(owned)
        .map_err(|_| format!("key {key} has unexpected type"))
}

fn dict_get_streams(
    dict: &HashMap<String, zbus::zvariant::OwnedValue>,
    key: &str,
) -> Result<Vec<(u32, HashMap<String, zbus::zvariant::OwnedValue>)>, String> {
    let value = dict.get(key).ok_or_else(|| format!("missing key {key}"))?;
    let owned = value
        .try_clone()
        .map_err(|_| format!("failed to clone key {key}"))?;
    Vec::try_from(owned).map_err(|_| format!("key {key} has unexpected type"))
}

fn token(kind: &str) -> String {
    format!("screen_record_{kind}_{}", std::process::id())
}

fn portal_options<const N: usize>(
    pairs: [(&'static str, zbus::zvariant::OwnedValue); N],
) -> HashMap<&'static str, zbus::zvariant::OwnedValue> {
    let mut out = HashMap::new();
    for (k, v) in pairs {
        out.insert(k, v);
    }
    out
}

fn portal_missing_error() -> CliError {
    CliError::runtime(
        "Wayland-only session detected but xdg-desktop-portal is missing.\n\
Install xdg-desktop-portal and a desktop backend (e.g. xdg-desktop-portal-gnome or xdg-desktop-portal-kde), then log out/in.\n\
Expected DBus service: org.freedesktop.portal.Desktop",
    )
}

fn env_flag_enabled(key: &str) -> bool {
    let Some(value) = env::var_os(key) else {
        return false;
    };
    let value = value.to_string_lossy();
    let normalized = value.trim().to_ascii_lowercase();
    matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
}
