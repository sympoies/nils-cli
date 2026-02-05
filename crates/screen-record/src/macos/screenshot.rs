use std::path::{Path, PathBuf};
use std::process::Command;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use block2::RcBlock;
use dispatch2::DispatchQueue;
use nils_common::process::find_in_path;
use objc2::rc::{autoreleasepool, Allocated, Retained};
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_core_foundation::{CFData, CFDictionary, CFNumber, CFRetained, CFString, CFType, CFURL};
use objc2_core_graphics::{
    kCGColorSpaceSRGB, CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGImage,
    CGImageAlphaInfo, CGImageByteOrderInfo,
};
use objc2_core_media::CMSampleBuffer;
use objc2_core_video::{
    kCVPixelFormatType_32BGRA, kCVPixelFormatType_32RGBA, kCVReturnSuccess, CVPixelBuffer,
    CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight,
    CVPixelBufferGetPixelFormatType, CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress,
    CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
};
use objc2_foundation::{NSDate, NSError, NSRunLoop};
use objc2_image_io::{kCGImageDestinationLossyCompressionQuality, CGImageDestination};
use objc2_screen_capture_kit::{
    SCContentFilter, SCShareableContent, SCStream, SCStreamConfiguration, SCStreamOutput,
    SCStreamOutputType, SCWindow,
};

use crate::cli::ImageFormat;
use crate::error::CliError;
use crate::types::WindowInfo;

pub fn screenshot_window(
    window: &WindowInfo,
    path: &Path,
    format: ImageFormat,
) -> Result<(), CliError> {
    autoreleasepool(|_| {
        let shareable = fetch_shareable_content()?;
        let sc_window = find_window(&shareable, window.id)?;

        let filter = unsafe {
            SCContentFilter::initWithDesktopIndependentWindow(SCContentFilter::alloc(), &sc_window)
        };
        let (width, height) = window_capture_dimensions(&sc_window, &filter)?;

        let config = unsafe { SCStreamConfiguration::new() };
        unsafe {
            config.setWidth(width as usize);
            config.setHeight(height as usize);
            config.setScalesToFit(true);
            config.setPreservesAspectRatio(true);
            config.setShowsCursor(true);
            config.setCapturesAudio(false);
            config.setMinimumFrameInterval(objc2_core_media::CMTime::new(1, 30));
        }

        let mtm = MainThreadMarker::new()
            .ok_or_else(|| CliError::runtime("screenshot capture must run on the main thread"))?;

        let capture_state = Rc::new(ScreenshotState::default());
        let output = ScreenshotOutput::new(capture_state.clone(), mtm);
        let output_proto = ProtocolObject::from_ref(&*output);

        let stream = unsafe {
            SCStream::initWithFilter_configuration_delegate(
                SCStream::alloc(),
                &filter,
                &config,
                None,
            )
        };
        unsafe {
            stream
                .addStreamOutput_type_sampleHandlerQueue_error(
                    output_proto,
                    SCStreamOutputType::Screen,
                    Some(DispatchQueue::main()),
                )
                .map_err(|err| ns_error_to_cli("failed to add stream output", &err))?;
        }

        start_capture(&stream)?;
        let sample_buffer = match wait_for_frame(&capture_state, Duration::from_secs(2))? {
            Some(buf) => buf,
            None => {
                stop_capture(&stream)?;
                return Err(CliError::runtime("timed out waiting for screenshot frame"));
            }
        };
        stop_capture(&stream)?;

        let frame = sample_buffer_to_rgba(&sample_buffer)?;
        write_frame_to_path(&frame, path, format)?;

        Ok(())
    })
}

#[derive(Default)]
struct ScreenshotState {
    frame: Mutex<Option<CFRetained<CMSampleBuffer>>>,
}

impl ScreenshotState {
    fn store_frame(&self, sample_buffer: &CMSampleBuffer) {
        let mut guard = match self.frame.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if guard.is_some() {
            return;
        }

        let ptr = NonNull::from(sample_buffer);
        let retained = unsafe { CFRetained::retain(ptr) };
        *guard = Some(retained);
    }

    fn take_frame(&self) -> Option<CFRetained<CMSampleBuffer>> {
        let mut guard = match self.frame.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.take()
    }
}

#[derive(Default)]
struct OutputIvars {
    capture: Mutex<Option<Rc<ScreenshotState>>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = OutputIvars]
    struct ScreenshotOutput;

    impl ScreenshotOutput {
        #[unsafe(method_id(init))]
        fn init(this: Allocated<Self>) -> Retained<Self> {
            let this = this.set_ivars(OutputIvars::default());
            unsafe { msg_send![super(this), init] }
        }
    }

    unsafe impl NSObjectProtocol for ScreenshotOutput {}

    unsafe impl SCStreamOutput for ScreenshotOutput {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        fn stream_did_output_sample_buffer_of_type(
            &self,
            _stream: &SCStream,
            sample_buffer: &CMSampleBuffer,
            r#type: SCStreamOutputType,
        ) {
            if r#type != SCStreamOutputType::Screen {
                return;
            }

            let guard = match self.ivars().capture.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            let Some(capture_state) = guard.as_ref() else {
                return;
            };

            capture_state.store_frame(sample_buffer);
        }
    }
);

impl ScreenshotOutput {
    fn new(capture_state: Rc<ScreenshotState>, mtm: MainThreadMarker) -> Retained<Self> {
        let output: Retained<Self> = unsafe { msg_send![ScreenshotOutput::alloc(mtm), init] };
        {
            let mut guard = match output.ivars().capture.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            *guard = Some(capture_state);
        }
        output
    }
}

fn wait_for_frame(
    capture_state: &ScreenshotState,
    timeout: Duration,
) -> Result<Option<CFRetained<CMSampleBuffer>>, CliError> {
    let run_loop = NSRunLoop::currentRunLoop();
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(frame) = capture_state.take_frame() {
            return Ok(Some(frame));
        }

        if Instant::now() >= deadline {
            return Ok(None);
        }

        let date = NSDate::dateWithTimeIntervalSinceNow(0.05);
        run_loop.runUntilDate(&date);
    }
}

fn fetch_shareable_content() -> Result<Retained<SCShareableContent>, CliError> {
    enum ShareableResult {
        Content(*mut SCShareableContent),
        Error(*mut NSError),
    }

    let (sender, receiver) = std::sync::mpsc::channel::<ShareableResult>();
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

    let result = wait_for_callback(&receiver, "shareable content")?;
    match result {
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

fn find_window(
    content: &SCShareableContent,
    window_id: u32,
) -> Result<Retained<SCWindow>, CliError> {
    let windows = unsafe { content.windows() };
    let count = windows.count();
    for idx in 0..count {
        let window = windows.objectAtIndex(idx);
        let id = unsafe { window.windowID() };
        if id == window_id {
            return Ok(window);
        }
    }
    Err(CliError::runtime(format!(
        "window id {window_id} is not available for capture"
    )))
}

fn window_capture_dimensions(
    window: &SCWindow,
    filter: &SCContentFilter,
) -> Result<(i32, i32), CliError> {
    let frame = unsafe { window.frame() };
    let scale = unsafe { filter.pointPixelScale() } as f64;
    let width = (frame.size.width * scale).round();
    let height = (frame.size.height * scale).round();
    if width <= 0.0 || height <= 0.0 {
        return Err(CliError::runtime("invalid window bounds"));
    }
    Ok((width as i32, height as i32))
}

fn start_capture(stream: &SCStream) -> Result<(), CliError> {
    let (sender, receiver) = std::sync::mpsc::channel::<Option<*mut NSError>>();
    let block = RcBlock::new(move |error: *mut NSError| {
        if error.is_null() {
            let _ = sender.send(None);
            return;
        }
        let retained = unsafe { Retained::retain_autoreleased(error) };
        let ptr = retained
            .map(Retained::into_raw)
            .unwrap_or_else(std::ptr::null_mut);
        let _ = sender.send(Some(ptr));
    });

    unsafe {
        stream.startCaptureWithCompletionHandler(Some(&block));
    }

    match wait_for_callback(&receiver, "start capture")? {
        None => Ok(()),
        Some(ptr) => {
            if ptr.is_null() {
                return Err(CliError::runtime("failed to start capture"));
            }
            let retained = unsafe { Retained::from_raw(ptr) }
                .ok_or_else(|| CliError::runtime("failed to retain start capture error"))?;
            Err(ns_error_to_cli("failed to start capture", &retained))
        }
    }
}

fn stop_capture(stream: &SCStream) -> Result<(), CliError> {
    let (sender, receiver) = std::sync::mpsc::channel::<Option<*mut NSError>>();
    let block = RcBlock::new(move |error: *mut NSError| {
        if error.is_null() {
            let _ = sender.send(None);
            return;
        }
        let retained = unsafe { Retained::retain_autoreleased(error) };
        let ptr = retained
            .map(Retained::into_raw)
            .unwrap_or_else(std::ptr::null_mut);
        let _ = sender.send(Some(ptr));
    });

    unsafe {
        stream.stopCaptureWithCompletionHandler(Some(&block));
    }

    match wait_for_callback(&receiver, "stop capture")? {
        None => Ok(()),
        Some(ptr) => {
            if ptr.is_null() {
                return Err(CliError::runtime("failed to stop capture"));
            }
            let retained = unsafe { Retained::from_raw(ptr) }
                .ok_or_else(|| CliError::runtime("failed to retain stop capture error"))?;
            Err(ns_error_to_cli("failed to stop capture", &retained))
        }
    }
}

fn wait_for_callback<T>(
    receiver: &std::sync::mpsc::Receiver<T>,
    label: &str,
) -> Result<T, CliError> {
    let run_loop = NSRunLoop::currentRunLoop();
    loop {
        match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(value) => return Ok(value),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                let date = NSDate::dateWithTimeIntervalSinceNow(0.05);
                run_loop.runUntilDate(&date);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
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

struct RgbaFrame {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
}

fn sample_buffer_to_rgba(sample_buffer: &CMSampleBuffer) -> Result<RgbaFrame, CliError> {
    let image_buffer = unsafe { sample_buffer.image_buffer() }
        .ok_or_else(|| CliError::runtime("missing image buffer"))?;

    let pixel_buffer: CFRetained<CVPixelBuffer> =
        unsafe { CFRetained::cast_unchecked(image_buffer) };
    let width = CVPixelBufferGetWidth(&pixel_buffer);
    let height = CVPixelBufferGetHeight(&pixel_buffer);
    if width == 0 || height == 0 {
        return Err(CliError::runtime("invalid frame dimensions"));
    }

    let lock_flags = CVPixelBufferLockFlags::ReadOnly;
    let lock_result = unsafe { CVPixelBufferLockBaseAddress(&pixel_buffer, lock_flags) };
    if lock_result != kCVReturnSuccess {
        return Err(CliError::runtime(format!(
            "failed to lock pixel buffer (CVReturn={lock_result})"
        )));
    }
    struct UnlockOnDrop<'a> {
        pb: &'a CVPixelBuffer,
        flags: CVPixelBufferLockFlags,
    }
    impl Drop for UnlockOnDrop<'_> {
        fn drop(&mut self) {
            let _ = unsafe { CVPixelBufferUnlockBaseAddress(self.pb, self.flags) };
        }
    }
    let _unlock = UnlockOnDrop {
        pb: &pixel_buffer,
        flags: lock_flags,
    };

    let base = CVPixelBufferGetBaseAddress(&pixel_buffer);
    if base.is_null() {
        return Err(CliError::runtime("pixel buffer base address is null"));
    }
    let bytes_per_row = CVPixelBufferGetBytesPerRow(&pixel_buffer);
    let pixel_format = CVPixelBufferGetPixelFormatType(&pixel_buffer);

    let src = base.cast::<u8>();
    let mut pixels = vec![0u8; width * height * 4];

    const PIXEL_FORMAT_BGRA: u32 = kCVPixelFormatType_32BGRA;
    const PIXEL_FORMAT_RGBA: u32 = kCVPixelFormatType_32RGBA;

    match pixel_format {
        PIXEL_FORMAT_BGRA => {
            for y in 0..height {
                let row_src = unsafe { src.add(y * bytes_per_row) };
                let row_dst = &mut pixels[y * width * 4..(y + 1) * width * 4];
                for x in 0..width {
                    let idx = x * 4;
                    let b = unsafe { *row_src.add(idx) };
                    let g = unsafe { *row_src.add(idx + 1) };
                    let r = unsafe { *row_src.add(idx + 2) };
                    let a = unsafe { *row_src.add(idx + 3) };
                    row_dst[idx] = r;
                    row_dst[idx + 1] = g;
                    row_dst[idx + 2] = b;
                    row_dst[idx + 3] = a;
                }
            }
        }
        PIXEL_FORMAT_RGBA => {
            for y in 0..height {
                let row_src_ptr = unsafe { src.add(y * bytes_per_row) };
                let row_dst = &mut pixels[y * width * 4..(y + 1) * width * 4];
                let row_src = unsafe { std::slice::from_raw_parts(row_src_ptr, width * 4) };
                row_dst.copy_from_slice(row_src);
            }
        }
        other => {
            return Err(CliError::runtime(format!(
                "unsupported pixel format: {other}"
            )));
        }
    }

    Ok(RgbaFrame {
        width,
        height,
        pixels,
    })
}

fn write_frame_to_path(
    frame: &RgbaFrame,
    path: &Path,
    format: ImageFormat,
) -> Result<(), CliError> {
    match format {
        ImageFormat::Png | ImageFormat::Jpg => write_via_imageio(frame, path, format),
        ImageFormat::Webp => {
            if write_via_imageio(frame, path, format).is_ok() {
                return Ok(());
            }
            write_webp_via_cwebp(frame, path)
        }
    }
}

fn write_via_imageio(frame: &RgbaFrame, path: &Path, format: ImageFormat) -> Result<(), CliError> {
    let parent = path
        .parent()
        .ok_or_else(|| CliError::runtime("missing output parent dir"))?;
    std::fs::create_dir_all(parent)
        .map_err(|err| CliError::runtime(format!("failed to create output dir: {err}")))?;

    let tmp = temp_path_for_target(path)?;
    let _tmp_guard = TempFileGuard::new(&tmp);

    encode_imageio_to_path(frame, &tmp, format)?;

    rename_overwrite(&tmp, path)
}

fn encode_imageio_to_path(
    frame: &RgbaFrame,
    path: &Path,
    format: ImageFormat,
) -> Result<(), CliError> {
    let url = CFURL::from_file_path(path)
        .ok_or_else(|| CliError::runtime("failed to create output URL"))?;

    let type_id = match format {
        ImageFormat::Png => CFString::from_static_str("public.png"),
        ImageFormat::Jpg => CFString::from_static_str("public.jpeg"),
        ImageFormat::Webp => CFString::from_static_str("org.webmproject.webp"),
    };

    let Some(dest) = (unsafe { CGImageDestination::with_url(&url, &type_id, 1, None) }) else {
        return Err(CliError::runtime(format!(
            "image encoder not available for {}",
            match format {
                ImageFormat::Png => "png",
                ImageFormat::Jpg => "jpg",
                ImageFormat::Webp => "webp",
            }
        )));
    };

    let alpha_info = match format {
        ImageFormat::Jpg => CGImageAlphaInfo::NoneSkipLast,
        _ => CGImageAlphaInfo::Last,
    };
    let image = cg_image_from_rgba(frame, alpha_info)?;

    let properties = if matches!(format, ImageFormat::Jpg) {
        // Default quality tuned for screenshots: visually good, still reasonably compact.
        let quality = CFNumber::new_f64(0.92);
        let key = unsafe { kCGImageDestinationLossyCompressionQuality };
        Some(CFDictionary::<CFType, CFType>::from_slices(
            &[key.as_ref()],
            &[quality.as_ref()],
        ))
    } else {
        None
    };

    unsafe {
        dest.add_image(&image, properties.as_ref().map(|dict| dict.as_ref()));
        if !dest.finalize() {
            return Err(CliError::runtime("failed to finalize image output"));
        }
    }

    Ok(())
}

fn cg_image_from_rgba(
    frame: &RgbaFrame,
    alpha: CGImageAlphaInfo,
) -> Result<CFRetained<CGImage>, CliError> {
    let data = CFData::from_bytes(&frame.pixels);
    let provider = CGDataProvider::with_cf_data(Some(data.as_ref()))
        .ok_or_else(|| CliError::runtime("failed to create CGDataProvider"))?;

    let space = unsafe { CGColorSpace::with_name(Some(kCGColorSpaceSRGB)) }
        .ok_or_else(|| CliError::runtime("failed to create sRGB colorspace"))?;

    let bitmap_info = CGBitmapInfo::from_bits_retain(alpha.0 | CGImageByteOrderInfo::Order32Big.0);

    unsafe {
        CGImage::new(
            frame.width,
            frame.height,
            8,
            32,
            frame.width * 4,
            Some(&space),
            bitmap_info,
            Some(&provider),
            std::ptr::null(),
            false,
            CGColorRenderingIntent::RenderingIntentDefault,
        )
        .ok_or_else(|| CliError::runtime("failed to create CGImage"))
    }
}

fn write_webp_via_cwebp(frame: &RgbaFrame, path: &Path) -> Result<(), CliError> {
    let cwebp = find_in_path("cwebp").ok_or_else(|| {
        CliError::runtime(
            "webp encoding not supported on this macOS. Install `cwebp` (brew install webp) or use --image-format png|jpg.",
        )
    })?;

    let parent = path
        .parent()
        .ok_or_else(|| CliError::runtime("missing output parent dir"))?;
    std::fs::create_dir_all(parent)
        .map_err(|err| CliError::runtime(format!("failed to create output dir: {err}")))?;

    let tmp_png = temp_path_for_target_with_suffix(path, "tmp.png")?;
    let _tmp_png_guard = TempFileGuard::new(&tmp_png);
    encode_imageio_to_path(frame, &tmp_png, ImageFormat::Png)?;

    let tmp_webp = temp_path_for_target_with_suffix(path, "tmp.webp")?;
    let _tmp_webp_guard = TempFileGuard::new(&tmp_webp);

    let out = Command::new(&cwebp)
        .args(["-lossless"])
        .arg(&tmp_png)
        .args(["-o"])
        .arg(&tmp_webp)
        .output()
        .map_err(|err| CliError::runtime(format!("failed to execute cwebp: {err}")))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(CliError::runtime(format!(
            "cwebp failed: {}",
            if stderr.is_empty() {
                "unknown error"
            } else {
                &stderr
            }
        )));
    }

    rename_overwrite(&tmp_webp, path)
}

fn rename_overwrite(from: &Path, to: &Path) -> Result<(), CliError> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(err) => {
            // On some platforms, rename may fail if the destination exists.
            if to.exists() {
                let _ = std::fs::remove_file(to);
            }
            std::fs::rename(from, to).map_err(|err2| {
                CliError::runtime(format!("failed to write output: {err} ({err2})"))
            })?;
            Ok(())
        }
    }
}

struct TempFileGuard {
    path: PathBuf,
}

impl TempFileGuard {
    fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
        }
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn temp_path_for_target(target: &Path) -> Result<PathBuf, CliError> {
    temp_path_for_target_with_suffix(target, "tmp")
}

fn temp_path_for_target_with_suffix(target: &Path, suffix: &str) -> Result<PathBuf, CliError> {
    let parent = target
        .parent()
        .ok_or_else(|| CliError::runtime("missing output parent dir"))?;
    let name = target
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("screenshot");
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    Ok(parent.join(format!(".{name}.{suffix}-{pid}-{nanos}")))
}
