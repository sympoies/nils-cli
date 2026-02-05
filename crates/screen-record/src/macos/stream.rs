use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use block2::RcBlock;
use dispatch2::DispatchQueue;
use objc2::rc::{autoreleasepool, Allocated, Retained};
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_av_foundation::{
    AVAuthorizationStatus, AVCaptureAudioDataOutput, AVCaptureAudioDataOutputSampleBufferDelegate,
    AVCaptureConnection, AVCaptureDevice, AVCaptureDeviceInput, AVCaptureOutput, AVCaptureSession,
    AVMediaTypeAudio,
};
use objc2_core_media::{CMSampleBuffer, CMTime};
use objc2_foundation::{NSDate, NSError, NSRunLoop};
use objc2_screen_capture_kit::{
    SCContentFilter, SCShareableContent, SCStream, SCStreamConfiguration, SCStreamOutput,
    SCStreamOutputType, SCWindow,
};

use crate::cli::{AudioMode, ContainerFormat};
use crate::error::CliError;
use crate::macos::writer::{AssetWriter, AudioConfig};
use crate::types::WindowInfo;

pub fn record_window(
    window: &WindowInfo,
    duration: u64,
    audio: AudioMode,
    path: &Path,
    format: ContainerFormat,
) -> Result<(), CliError> {
    autoreleasepool(|_| {
        let shareable = fetch_shareable_content()?;
        let sc_window = find_window(&shareable, window.id)?;
        let captures_system_audio = matches!(audio, AudioMode::System | AudioMode::Both);
        let captures_mic_audio = matches!(audio, AudioMode::Mic | AudioMode::Both);

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
            config.setCapturesAudio(captures_system_audio);
            if captures_system_audio {
                config.setSampleRate(48_000);
                config.setChannelCount(2);
            }
            config.setMinimumFrameInterval(CMTime::new(1, 30));
        }

        let audio_config = AudioConfig {
            system: captures_system_audio,
            mic: captures_mic_audio,
        };
        let writer = AssetWriter::new(path, format, width, height, audio_config)?;
        let capture_state = Rc::new(CaptureState::new(writer));
        let mtm = MainThreadMarker::new()
            .ok_or_else(|| CliError::runtime("screen recording must run on the main thread"))?;
        let output = StreamOutput::new(capture_state.clone(), mtm);
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
        if captures_system_audio {
            unsafe {
                stream
                    .addStreamOutput_type_sampleHandlerQueue_error(
                        output_proto,
                        SCStreamOutputType::Audio,
                        Some(DispatchQueue::main()),
                    )
                    .map_err(|err| ns_error_to_cli("failed to add audio output", &err))?;
            }
        }

        start_capture(&stream)?;
        let mic_capture = if captures_mic_audio {
            Some(MicCapture::start(capture_state.clone())?)
        } else {
            None
        };

        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop_handle = stop_flag.clone();
        ctrlc::set_handler(move || {
            stop_handle.store(true, Ordering::SeqCst);
        })
        .map_err(|err| CliError::runtime(format!("failed to set Ctrl-C handler: {err}")))?;

        run_capture_loop(Duration::from_secs(duration), &stop_flag);

        stop_capture(&stream)?;
        if let Some(mic_capture) = mic_capture.as_ref() {
            mic_capture.stop();
        }

        let captured_error = output.take_error();
        let finish_result = output.finish();
        if let Some(err) = captured_error {
            return Err(err);
        }
        finish_result?;

        Ok(())
    })
}

struct CaptureState {
    writer: Mutex<Option<AssetWriter>>,
    error: Mutex<Option<CliError>>,
}

impl CaptureState {
    fn new(writer: AssetWriter) -> Self {
        Self {
            writer: Mutex::new(Some(writer)),
            error: Mutex::new(None),
        }
    }

    fn append_video(&self, sample_buffer: &CMSampleBuffer) {
        self.append(|writer| writer.append_video_sample_buffer(sample_buffer));
    }

    fn append_system_audio(&self, sample_buffer: &CMSampleBuffer) {
        self.append(|writer| writer.append_system_audio_sample_buffer(sample_buffer));
    }

    fn append_mic_audio(&self, sample_buffer: &CMSampleBuffer) {
        self.append(|writer| writer.append_mic_audio_sample_buffer(sample_buffer));
    }

    fn append<F>(&self, append_fn: F)
    where
        F: FnOnce(&mut AssetWriter) -> Result<(), CliError>,
    {
        if self.has_error() {
            return;
        }

        let writer_lock = self.writer.lock();
        let mut writer_guard = match writer_lock {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(writer) = writer_guard.as_mut() {
            if let Err(err) = append_fn(writer) {
                self.set_error(err);
            }
        }
    }

    fn finish(&self) -> Result<(), CliError> {
        let writer_lock = self.writer.lock();
        let mut writer_guard = match writer_lock {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(writer) = writer_guard.take() {
            return writer.finish();
        }
        Ok(())
    }

    fn take_error(&self) -> Option<CliError> {
        let error_lock = self.error.lock();
        let mut error_guard = match error_lock {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        error_guard.take()
    }

    fn has_error(&self) -> bool {
        let error_lock = self.error.lock();
        let error_guard = match error_lock {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        error_guard.is_some()
    }

    fn set_error(&self, err: CliError) {
        let error_lock = self.error.lock();
        let mut error_guard = match error_lock {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if error_guard.is_none() {
            *error_guard = Some(err);
        }
    }
}

#[derive(Default)]
struct StreamState {
    capture: Mutex<Option<Rc<CaptureState>>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = StreamState]
    struct StreamOutput;

    impl StreamOutput {
        #[unsafe(method_id(init))]
        fn init(this: Allocated<Self>) -> Retained<Self> {
            let this = this.set_ivars(StreamState::default());
            unsafe { msg_send![super(this), init] }
        }
    }

    unsafe impl NSObjectProtocol for StreamOutput {}

    unsafe impl SCStreamOutput for StreamOutput {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        fn stream_did_output_sample_buffer_of_type(
            &self,
            _stream: &SCStream,
            sample_buffer: &CMSampleBuffer,
            r#type: SCStreamOutputType,
        ) {
            let capture_lock = self.ivars().capture.lock();
            let capture_guard = match capture_lock {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            let Some(capture_state) = capture_guard.as_ref() else {
                return;
            };

            match r#type {
                SCStreamOutputType::Screen => capture_state.append_video(sample_buffer),
                SCStreamOutputType::Audio => capture_state.append_system_audio(sample_buffer),
                _ => {}
            }
        }
    }
);

impl StreamOutput {
    fn new(capture_state: Rc<CaptureState>, mtm: MainThreadMarker) -> Retained<Self> {
        let output: Retained<Self> = unsafe { msg_send![StreamOutput::alloc(mtm), init] };
        {
            let capture_lock = output.ivars().capture.lock();
            let mut capture_guard = match capture_lock {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            *capture_guard = Some(capture_state);
        }
        output
    }

    fn finish(&self) -> Result<(), CliError> {
        let capture_lock = self.ivars().capture.lock();
        let capture_guard = match capture_lock {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let Some(capture_state) = capture_guard.as_ref() else {
            return Ok(());
        };
        capture_state.finish()
    }

    fn take_error(&self) -> Option<CliError> {
        let capture_lock = self.ivars().capture.lock();
        let capture_guard = match capture_lock {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let capture_state = capture_guard.as_ref()?;
        capture_state.take_error()
    }
}

#[derive(Default)]
struct MicState {
    capture: Mutex<Option<Rc<CaptureState>>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = MicState]
    struct MicOutput;

    impl MicOutput {
        #[unsafe(method_id(init))]
        fn init(this: Allocated<Self>) -> Retained<Self> {
            let this = this.set_ivars(MicState::default());
            unsafe { msg_send![super(this), init] }
        }
    }

    unsafe impl NSObjectProtocol for MicOutput {}

    unsafe impl AVCaptureAudioDataOutputSampleBufferDelegate for MicOutput {
        #[unsafe(method(captureOutput:didOutputSampleBuffer:fromConnection:))]
        fn capture_output_did_output_sample_buffer_from_connection(
            &self,
            _output: &AVCaptureOutput,
            sample_buffer: &CMSampleBuffer,
            _connection: &AVCaptureConnection,
        ) {
            let capture_lock = self.ivars().capture.lock();
            let capture_guard = match capture_lock {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            let Some(capture_state) = capture_guard.as_ref() else {
                return;
            };
            capture_state.append_mic_audio(sample_buffer);
        }
    }
);

impl MicOutput {
    fn new(capture_state: Rc<CaptureState>, mtm: MainThreadMarker) -> Retained<Self> {
        let output: Retained<Self> = unsafe { msg_send![MicOutput::alloc(mtm), init] };
        {
            let capture_lock = output.ivars().capture.lock();
            let mut capture_guard = match capture_lock {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            *capture_guard = Some(capture_state);
        }
        output
    }
}

struct MicCapture {
    session: Retained<AVCaptureSession>,
    output: Retained<AVCaptureAudioDataOutput>,
    _input: Retained<AVCaptureDeviceInput>,
    _delegate: Retained<MicOutput>,
}

impl MicCapture {
    fn start(capture_state: Rc<CaptureState>) -> Result<Self, CliError> {
        let media_type = unsafe { AVMediaTypeAudio }
            .ok_or_else(|| CliError::runtime("missing AVFoundation audio media type"))?;
        let status = unsafe { AVCaptureDevice::authorizationStatusForMediaType(media_type) };
        if matches!(
            status,
            AVAuthorizationStatus::Denied | AVAuthorizationStatus::Restricted
        ) {
            return Err(CliError::runtime(
                "microphone access denied. Enable in System Settings > Privacy & Security > Microphone.",
            ));
        }

        let device = unsafe { AVCaptureDevice::defaultDeviceWithMediaType(media_type) }
            .ok_or_else(|| CliError::runtime("no microphone device available"))?;
        let input = unsafe { AVCaptureDeviceInput::deviceInputWithDevice_error(&device) }
            .map_err(|err| ns_error_to_cli("failed to open microphone device", &err))?;

        let session = unsafe { AVCaptureSession::new() };
        let output = unsafe { AVCaptureAudioDataOutput::new() };

        if !unsafe { session.canAddInput(&input) } {
            return Err(CliError::runtime("failed to add microphone input"));
        }
        if !unsafe { session.canAddOutput(&output) } {
            return Err(CliError::runtime("failed to add microphone output"));
        }

        unsafe { session.addInput(&input) };
        unsafe { session.addOutput(&output) };

        let mtm = MainThreadMarker::new()
            .ok_or_else(|| CliError::runtime("microphone capture must run on the main thread"))?;
        let delegate = MicOutput::new(capture_state, mtm);
        let delegate_proto = ProtocolObject::from_ref(&*delegate);
        unsafe {
            output.setSampleBufferDelegate_queue(Some(delegate_proto), Some(DispatchQueue::main()));
        }

        unsafe { session.startRunning() };

        Ok(Self {
            session,
            output,
            _input: input,
            _delegate: delegate,
        })
    }

    fn stop(&self) {
        unsafe {
            self.output.setSampleBufferDelegate_queue(None, None);
            self.session.stopRunning();
        }
    }
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
    let (sender, receiver) = mpsc::channel::<Option<*mut NSError>>();
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
    let (sender, receiver) = mpsc::channel::<Option<*mut NSError>>();
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

fn run_capture_loop(duration: Duration, stop_flag: &Arc<AtomicBool>) {
    let run_loop = NSRunLoop::currentRunLoop();
    let deadline = Instant::now() + duration;
    loop {
        if stop_flag.load(Ordering::SeqCst) {
            break;
        }

        let now = Instant::now();
        if now >= deadline {
            break;
        }

        let remaining = deadline - now;
        let step = remaining.min(Duration::from_millis(100));
        let date = NSDate::dateWithTimeIntervalSinceNow(step.as_secs_f64());
        run_loop.runUntilDate(&date);
    }
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
