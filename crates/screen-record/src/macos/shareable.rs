use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::{Retained, autoreleasepool};
use objc2_foundation::{NSDate, NSError, NSRunLoop};
use objc2_screen_capture_kit::{SCRunningApplication, SCShareableContent};

use crate::error::CliError;
use crate::types::{AppInfo, DisplayInfo, Rect, ShareableContent, WindowInfo};

pub fn fetch_shareable() -> Result<ShareableContent, CliError> {
    let content = fetch_shareable_content()?;
    let displays = extract_displays(&content);
    let windows = extract_windows(&content);
    let apps = extract_apps(&content);

    Ok(ShareableContent {
        displays,
        windows,
        apps,
    })
}

fn fetch_shareable_content() -> Result<Retained<SCShareableContent>, CliError> {
    enum ShareableResult {
        Content(*mut SCShareableContent),
        Error(*mut NSError),
    }

    let (sender, receiver) = mpsc::channel::<ShareableResult>();
    let block = RcBlock::new(
        move |content: *mut SCShareableContent, error: *mut NSError| {
            if !error.is_null() {
                let retained = unsafe { Retained::retain_autoreleased(error) };
                let ptr = retained
                    .map(Retained::into_raw)
                    .unwrap_or_else(std::ptr::null_mut);
                let _ = sender.send(ShareableResult::Error(ptr));
                return;
            }

            if content.is_null() {
                let _ = sender.send(ShareableResult::Error(std::ptr::null_mut()));
                return;
            }

            let retained = unsafe { Retained::retain_autoreleased(content) };
            let ptr = retained
                .map(Retained::into_raw)
                .unwrap_or_else(std::ptr::null_mut);
            let _ = sender.send(ShareableResult::Content(ptr));
        },
    );

    unsafe {
        SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(
            true,
            true,
            &block,
        );
    }

    match wait_for_callback(&receiver, "shareable content")? {
        ShareableResult::Content(ptr) => {
            let retained = unsafe { Retained::from_raw(ptr) }
                .ok_or_else(|| CliError::runtime("failed to retain shareable content"))?;
            Ok(retained)
        }
        ShareableResult::Error(ptr) => {
            if ptr.is_null() {
                return Err(CliError::runtime("failed to fetch shareable content"));
            }
            let retained = unsafe { Retained::from_raw(ptr) }
                .ok_or_else(|| CliError::runtime("failed to retain shareable content error"))?;
            Err(ns_error_to_cli(
                "failed to fetch shareable content",
                &retained,
            ))
        }
    }
}

fn extract_windows(content: &SCShareableContent) -> Vec<WindowInfo> {
    let windows = unsafe { content.windows() };
    let count = windows.count();
    autoreleasepool(|pool| {
        let mut list = Vec::with_capacity(count);
        for idx in 0..count {
            let window = windows.objectAtIndex(idx);
            let id = unsafe { window.windowID() };
            let on_screen = unsafe { window.isOnScreen() };
            let active = unsafe { window.isActive() };
            let title = unsafe { window.title() }
                .map(|value| unsafe { value.to_str(pool) }.to_string())
                .unwrap_or_default();

            let (owner_name, owner_pid) = match unsafe { window.owningApplication() } {
                Some(app) => {
                    let name = unsafe { app.applicationName() };
                    let name = unsafe { name.to_str(pool) }.to_string();
                    let pid = unsafe { app.processID() };
                    (name, pid)
                }
                None => (String::new(), 0),
            };

            let frame = unsafe { window.frame() };
            let bounds = Rect {
                x: frame.origin.x.round() as i32,
                y: frame.origin.y.round() as i32,
                width: frame.size.width.round() as i32,
                height: frame.size.height.round() as i32,
            };

            list.push(WindowInfo {
                id,
                owner_name,
                title,
                bounds,
                on_screen,
                active,
                owner_pid,
                z_order: idx,
            });
        }
        list
    })
}

fn extract_displays(content: &SCShareableContent) -> Vec<DisplayInfo> {
    let displays = unsafe { content.displays() };
    let count = displays.count();
    let mut list = Vec::with_capacity(count);
    for idx in 0..count {
        let display = displays.objectAtIndex(idx);
        let id = unsafe { display.displayID() };
        let width = unsafe { display.width() } as i32;
        let height = unsafe { display.height() } as i32;

        list.push(DisplayInfo { id, width, height });
    }
    list
}

fn extract_apps(content: &SCShareableContent) -> Vec<AppInfo> {
    let apps = unsafe { content.applications() };
    let count = apps.count();
    autoreleasepool(|pool| {
        let mut list = Vec::with_capacity(count);
        for idx in 0..count {
            let app: Retained<SCRunningApplication> = apps.objectAtIndex(idx);
            let name = unsafe { app.applicationName() };
            let bundle = unsafe { app.bundleIdentifier() };
            let pid = unsafe { app.processID() };

            list.push(AppInfo {
                name: unsafe { name.to_str(pool) }.to_string(),
                pid,
                bundle_id: unsafe { bundle.to_str(pool) }.to_string(),
            });
        }
        list
    })
}

fn wait_for_callback<T>(receiver: &mpsc::Receiver<T>, label: &str) -> Result<T, CliError> {
    let run_loop = NSRunLoop::currentRunLoop();
    loop {
        match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(value) => return Ok(value),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let date = NSDate::dateWithTimeIntervalSinceNow(0.05);
                run_loop.runUntilDate(&date);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(CliError::runtime(format!(
                    "{label} completion channel disconnected"
                )));
            }
        }
    }
}

fn ns_error_to_cli(prefix: &str, error: &NSError) -> CliError {
    let description = ns_error_description(error);
    CliError::runtime(format!("{prefix}: {description}"))
}

fn ns_error_description(error: &NSError) -> String {
    autoreleasepool(|pool| {
        let description = error.localizedDescription();
        unsafe { description.to_str(pool) }.to_string()
    })
}
