use std::io::Read;
use std::os::unix::io::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::randr::{self, ConnectionExt as RandrExt, Output};
use x11rb::protocol::xproto::Timestamp;

use super::audio;
use super::portal::PortalCapture;
use crate::cli::{AudioMode, ContainerFormat, ImageFormat};
use crate::error::CliError;
use crate::types::WindowInfo;

const CAPTURE_FRAMERATE: u32 = 30;
const STDERR_LIMIT_BYTES: usize = 32 * 1024;
const KILL_GRACE: Duration = Duration::from_secs(10);

static CTRL_C_INSTALLED: OnceLock<Result<(), CliError>> = OnceLock::new();
static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy)]
struct DisplayBounds {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

pub fn record_window(
    window: &WindowInfo,
    duration: u64,
    audio: AudioMode,
    path: &Path,
    format: ContainerFormat,
) -> Result<(), CliError> {
    let display = x11_display()?;
    let pulse = audio::resolve_pulse_inputs(audio)?;
    let mut cmd = ffmpeg_base();
    cmd.args(["-f", "x11grab"])
        .args(["-framerate", CAPTURE_FRAMERATE.to_string().as_str()])
        .args(["-draw_mouse", "1"])
        .args(["-window_id", &format!("0x{:x}", window.id)])
        .args(["-i", &display]);
    append_pulse_inputs(&mut cmd, &pulse.sources);
    cmd.args(["-t", &duration.to_string()]);
    apply_stream_mapping(&mut cmd, pulse.sources.len());
    apply_video_encoding(&mut cmd, format);
    apply_audio_encoding(&mut cmd, pulse.sources.len());
    cmd.args(["-f", container_muxer(format)]);
    cmd.arg(path);
    run_ffmpeg(cmd)
}

pub fn record_portal_node(
    node_id: u32,
    duration: u64,
    path: &Path,
    format: ContainerFormat,
) -> Result<(), CliError> {
    ensure_pipewire_supported()?;

    let mut cmd = ffmpeg_base();
    cmd.args(["-f", "pipewire"])
        .args(["-i", &node_id.to_string()])
        .args(["-t", &duration.to_string()]);
    apply_video_encoding(&mut cmd, format);
    cmd.args(["-f", container_muxer(format)]);
    cmd.arg(path);
    run_ffmpeg(cmd)
}

pub fn record_portal(
    capture: &PortalCapture,
    duration: u64,
    path: &Path,
    format: ContainerFormat,
) -> Result<(), CliError> {
    if capture.pipewire_remote.is_none() {
        return record_portal_node(capture.node_id, duration, path, format);
    }

    ensure_pipewire_supported()?;
    let fd_flag = detect_pipewire_fd_flag()?.ok_or_else(|| {
        CliError::runtime(
            "ffmpeg PipeWire input does not appear to support portal FDs (missing -pipewire_fd in `ffmpeg -hide_banner -h demuxer=pipewire`). Install an ffmpeg build with portal/PipeWire FD support or switch to an Xorg session.",
        )
    })?;

    let pipewire_fd = capture
        .pipewire_remote
        .as_ref()
        .expect("checked")
        .as_raw_fd();

    let target_fd: i32 = 3;
    let mut cmd = ffmpeg_base();
    cmd.args(["-f", "pipewire"])
        .args([fd_flag, &target_fd.to_string()])
        .args(["-i", &capture.node_id.to_string()])
        .args(["-t", &duration.to_string()]);
    apply_video_encoding(&mut cmd, format);
    cmd.args(["-f", container_muxer(format)]);
    cmd.arg(path);

    unsafe {
        cmd.pre_exec(move || {
            if libc::dup2(pipewire_fd, target_fd) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::fcntl(target_fd, libc::F_SETFD, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    run_ffmpeg(cmd)
}

pub fn screenshot_window(
    window: &WindowInfo,
    path: &Path,
    _format: ImageFormat,
) -> Result<(), CliError> {
    let display = x11_display()?;

    let mut cmd = ffmpeg_base();
    cmd.args(["-f", "x11grab"])
        .args(["-draw_mouse", "1"])
        .args(["-window_id", &format!("0x{:x}", window.id)])
        .args(["-i", &display])
        .args(["-frames:v", "1"]);
    cmd.arg(path);

    run_ffmpeg(cmd)
}

pub fn screenshot_portal(
    capture: &PortalCapture,
    path: &Path,
    _format: ImageFormat,
) -> Result<(), CliError> {
    if capture.pipewire_remote.is_none() {
        return screenshot_portal_node(capture.node_id, path);
    }

    ensure_pipewire_supported()?;
    let fd_flag = detect_pipewire_fd_flag()?.ok_or_else(|| {
        CliError::runtime(
            "ffmpeg PipeWire input does not appear to support portal FDs (missing -pipewire_fd in `ffmpeg -hide_banner -h demuxer=pipewire`). Install an ffmpeg build with portal/PipeWire FD support or switch to an Xorg session.",
        )
    })?;

    let pipewire_fd = capture
        .pipewire_remote
        .as_ref()
        .expect("checked")
        .as_raw_fd();

    let target_fd: i32 = 3;
    let mut cmd = ffmpeg_base();
    cmd.args(["-f", "pipewire"])
        .args([fd_flag, &target_fd.to_string()])
        .args(["-i", &capture.node_id.to_string()])
        .args(["-frames:v", "1"]);
    cmd.arg(path);

    unsafe {
        cmd.pre_exec(move || {
            if libc::dup2(pipewire_fd, target_fd) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::fcntl(target_fd, libc::F_SETFD, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    run_ffmpeg(cmd)
}

fn screenshot_portal_node(node_id: u32, path: &Path) -> Result<(), CliError> {
    ensure_pipewire_supported()?;
    let mut cmd = ffmpeg_base();
    cmd.args(["-f", "pipewire"])
        .args(["-i", &node_id.to_string()])
        .args(["-frames:v", "1"]);
    cmd.arg(path);
    run_ffmpeg(cmd)
}

pub fn record_display(
    display_id: u32,
    duration: u64,
    audio: AudioMode,
    path: &Path,
    format: ContainerFormat,
) -> Result<(), CliError> {
    let display = x11_display()?;
    let pulse = audio::resolve_pulse_inputs(audio)?;
    let bounds = resolve_display_bounds(display_id)?;

    let mut cmd = ffmpeg_base();
    cmd.args(["-f", "x11grab"])
        .args(["-framerate", CAPTURE_FRAMERATE.to_string().as_str()])
        .args(["-draw_mouse", "1"])
        .args([
            "-video_size",
            &format!("{}x{}", bounds.width, bounds.height),
        ])
        .args(["-i", &format!("{display}+{},{}", bounds.x, bounds.y)]);
    append_pulse_inputs(&mut cmd, &pulse.sources);
    cmd.args(["-t", &duration.to_string()]);
    apply_stream_mapping(&mut cmd, pulse.sources.len());
    apply_video_encoding(&mut cmd, format);
    apply_audio_encoding(&mut cmd, pulse.sources.len());
    cmd.args(["-f", container_muxer(format)]);
    cmd.arg(path);
    run_ffmpeg(cmd)
}

pub fn record_main_display(
    duration: u64,
    audio: AudioMode,
    path: &Path,
    format: ContainerFormat,
) -> Result<(), CliError> {
    let display = x11_display()?;
    let pulse = audio::resolve_pulse_inputs(audio)?;
    let bounds = resolve_main_display_bounds()?;

    let mut cmd = ffmpeg_base();
    cmd.args(["-f", "x11grab"])
        .args(["-framerate", CAPTURE_FRAMERATE.to_string().as_str()])
        .args(["-draw_mouse", "1"])
        .args([
            "-video_size",
            &format!("{}x{}", bounds.width, bounds.height),
        ])
        .args(["-i", &format!("{display}+{},{}", bounds.x, bounds.y)]);
    append_pulse_inputs(&mut cmd, &pulse.sources);
    cmd.args(["-t", &duration.to_string()]);
    apply_stream_mapping(&mut cmd, pulse.sources.len());
    apply_video_encoding(&mut cmd, format);
    apply_audio_encoding(&mut cmd, pulse.sources.len());
    cmd.args(["-f", container_muxer(format)]);
    cmd.arg(path);
    run_ffmpeg(cmd)
}

fn ffmpeg_base() -> Command {
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"]);
    cmd
}

fn ensure_pipewire_supported() -> Result<(), CliError> {
    let supported = detect_pipewire_supported()?;

    if supported {
        return Ok(());
    }

    Err(CliError::runtime(
        "ffmpeg does not support PipeWire input (missing \"pipewire\" in `ffmpeg -hide_banner -devices`). Install an ffmpeg build with PipeWire support or record via X11 (log into an Xorg session).",
    ))
}

fn detect_pipewire_supported() -> Result<bool, CliError> {
    let output = Command::new("ffmpeg")
        .args(["-hide_banner", "-devices"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(map_spawn_error)?;

    if !output.status.success() {
        let mut bytes = output.stderr;
        bytes.extend_from_slice(&output.stdout);
        let snippet = stderr_snippet(&bytes);
        return Err(CliError::runtime(format!(
            "ffmpeg -devices failed{}{}",
            exit_status_suffix(output.status),
            snippet
        )));
    }

    let mut text = String::new();
    text.push_str(&String::from_utf8_lossy(&output.stdout));
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok(text
        .lines()
        .any(|line| line.to_ascii_lowercase().contains("pipewire")))
}

fn detect_pipewire_fd_flag() -> Result<Option<&'static str>, CliError> {
    let output = Command::new("ffmpeg")
        .args(["-hide_banner", "-h", "demuxer=pipewire"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(map_spawn_error)?;

    if !output.status.success() {
        let mut bytes = output.stderr;
        bytes.extend_from_slice(&output.stdout);
        let snippet = stderr_snippet(&bytes);
        return Err(CliError::runtime(format!(
            "ffmpeg -h demuxer=pipewire failed{}{}",
            exit_status_suffix(output.status),
            snippet
        )));
    }

    let mut text = String::new();
    text.push_str(&String::from_utf8_lossy(&output.stdout));
    text.push_str(&String::from_utf8_lossy(&output.stderr));

    if text.contains("-pipewire_fd") {
        return Ok(Some("-pipewire_fd"));
    }
    Ok(None)
}

fn apply_video_encoding(cmd: &mut Command, format: ContainerFormat) {
    cmd.args(["-c:v", "libx264"])
        .args(["-preset", "ultrafast"])
        .args(["-crf", "23"])
        .args(["-pix_fmt", "yuv420p"])
        .args(["-vf", "pad=ceil(iw/2)*2:ceil(ih/2)*2"]);

    if format == ContainerFormat::Mp4 {
        cmd.args(["-movflags", "+faststart"]);
    }
}

fn apply_audio_encoding(cmd: &mut Command, audio_count: usize) {
    if audio_count == 0 {
        return;
    }
    cmd.args(["-c:a", "aac"]);
}

fn apply_stream_mapping(cmd: &mut Command, audio_count: usize) {
    cmd.args(["-map", "0:v:0"]);
    for idx in 0..audio_count {
        cmd.args(["-map", &format!("{}:a:0", idx + 1)]);
    }
}

fn append_pulse_inputs(cmd: &mut Command, sources: &[String]) {
    for source in sources {
        cmd.args(["-f", "pulse"]).args(["-i", source]);
    }
}

fn container_muxer(format: ContainerFormat) -> &'static str {
    match format {
        ContainerFormat::Mov => "mov",
        ContainerFormat::Mp4 => "mp4",
    }
}

fn x11_display() -> Result<String, CliError> {
    let display = std::env::var("DISPLAY")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| CliError::runtime("X11 display not detected (DISPLAY is unset)."))?;
    Ok(display)
}

fn resolve_display_bounds(display_id: u32) -> Result<DisplayBounds, CliError> {
    let (conn, screen_num) = x11rb::connect(None)
        .map_err(|err| CliError::runtime(format!("failed to connect to X11: {err}")))?;
    let setup = conn.setup();
    let screen = setup
        .roots
        .get(screen_num)
        .ok_or_else(|| CliError::runtime("failed to resolve X11 screen"))?;
    let root_bounds = DisplayBounds {
        x: 0,
        y: 0,
        width: screen.width_in_pixels as u32,
        height: screen.height_in_pixels as u32,
    };

    let root = screen.root;
    let resources = match conn.randr_get_screen_resources_current(root) {
        Ok(cookie) => cookie.reply().ok(),
        Err(_) => None,
    };
    let Some(resources) = resources else {
        if display_id == 1 {
            return Ok(root_bounds);
        }
        return Err(CliError::runtime(format!(
            "display id {display_id} not found"
        )));
    };

    if let Some(bounds) = bounds_for_output(&conn, resources.config_timestamp, display_id) {
        return Ok(bounds);
    }

    if display_id == 1 {
        Ok(root_bounds)
    } else {
        Err(CliError::runtime(format!(
            "display id {display_id} not found"
        )))
    }
}

fn resolve_main_display_bounds() -> Result<DisplayBounds, CliError> {
    let (conn, screen_num) = x11rb::connect(None)
        .map_err(|err| CliError::runtime(format!("failed to connect to X11: {err}")))?;
    let setup = conn.setup();
    let screen = setup
        .roots
        .get(screen_num)
        .ok_or_else(|| CliError::runtime("failed to resolve X11 screen"))?;
    let root_bounds = DisplayBounds {
        x: 0,
        y: 0,
        width: screen.width_in_pixels as u32,
        height: screen.height_in_pixels as u32,
    };

    let root = screen.root;
    let resources = match conn.randr_get_screen_resources_current(root) {
        Ok(cookie) => cookie.reply().ok(),
        Err(_) => None,
    };
    let Some(resources) = resources else {
        return Ok(root_bounds);
    };

    let primary_output = match conn.randr_get_output_primary(root) {
        Ok(cookie) => cookie.reply().ok().map(|reply| reply.output),
        Err(_) => None,
    }
    .filter(|output| *output != 0);

    if let Some(output) = primary_output
        && let Some(bounds) = bounds_for_output(&conn, resources.config_timestamp, output)
    {
        return Ok(bounds);
    }

    let mut candidates: Vec<Output> = resources.outputs;
    candidates.sort_unstable();
    for output in candidates {
        if let Some(bounds) = bounds_for_output(&conn, resources.config_timestamp, output) {
            return Ok(bounds);
        }
    }

    Ok(root_bounds)
}

fn bounds_for_output<C: Connection>(
    conn: &C,
    timestamp: Timestamp,
    output: Output,
) -> Option<DisplayBounds> {
    let output_info = conn
        .randr_get_output_info(output, timestamp)
        .ok()?
        .reply()
        .ok()?;
    if output_info.connection != randr::Connection::CONNECTED {
        return None;
    }
    if output_info.crtc == 0 {
        return None;
    }

    let crtc_info = conn
        .randr_get_crtc_info(output_info.crtc, timestamp)
        .ok()?
        .reply()
        .ok()?;
    if crtc_info.width == 0 || crtc_info.height == 0 {
        return None;
    }

    Some(DisplayBounds {
        x: crtc_info.x as i32,
        y: crtc_info.y as i32,
        width: crtc_info.width as u32,
        height: crtc_info.height as u32,
    })
}

fn run_ffmpeg(mut cmd: Command) -> Result<(), CliError> {
    install_ctrlc_handler()?;
    STOP_REQUESTED.store(false, Ordering::SeqCst);

    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(map_spawn_error)?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| CliError::runtime("failed to capture ffmpeg stderr"))?;
    let stderr_handle = std::thread::spawn(move || read_bounded(&mut stderr, STDERR_LIMIT_BYTES));

    let mut stop_instant: Option<Instant> = None;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(err) => {
                return Err(CliError::runtime(format!(
                    "failed to wait for ffmpeg: {err}"
                )))
            }
        }

        if STOP_REQUESTED.load(Ordering::SeqCst) && stop_instant.is_none() {
            stop_instant = Some(Instant::now());
        }

        if let Some(start) = stop_instant
            && start.elapsed() >= KILL_GRACE
        {
            let _ = child.kill();
        }

        std::thread::sleep(Duration::from_millis(25));
    };

    let stderr_bytes = stderr_handle.join().unwrap_or_default();

    if status.success() {
        return Ok(());
    }

    Err(CliError::runtime(format!(
        "ffmpeg failed{}{}",
        exit_status_suffix(status),
        stderr_snippet(&stderr_bytes)
    )))
}

fn install_ctrlc_handler() -> Result<(), CliError> {
    CTRL_C_INSTALLED
        .get_or_init(|| {
            ctrlc::set_handler(|| {
                STOP_REQUESTED.store(true, Ordering::SeqCst);
            })
            .map_err(|err| CliError::runtime(format!("failed to set Ctrl-C handler: {err}")))?;
            Ok(())
        })
        .clone()
}

fn read_bounded(reader: &mut impl Read, limit: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        out.extend_from_slice(&buf[..n]);
        if out.len() > limit {
            let trim = out.len() - limit;
            out.drain(0..trim);
        }
    }
    out
}

fn stderr_snippet(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    let text = String::from_utf8_lossy(bytes);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    format!(": {trimmed}")
}

fn exit_status_suffix(status: ExitStatus) -> String {
    if let Some(code) = status.code() {
        return format!(" (exit code {code})");
    }
    String::new()
}

fn map_spawn_error(err: std::io::Error) -> CliError {
    if err.kind() == std::io::ErrorKind::NotFound {
        return CliError::runtime(
            "ffmpeg not found on PATH. Install it with: sudo apt-get install ffmpeg",
        );
    }
    CliError::runtime(format!("failed to spawn ffmpeg: {err}"))
}
