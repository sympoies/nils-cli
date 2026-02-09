use std::path::Path;
use std::time::Duration;

use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::AnyObject;
use objc2_av_foundation::{
    AVAssetWriter, AVAssetWriterInput, AVFileTypeMPEG4, AVFileTypeQuickTimeMovie, AVMediaTypeAudio,
    AVMediaTypeVideo, AVVideoCodecKey, AVVideoCodecTypeH264, AVVideoHeightKey, AVVideoWidthKey,
};
use objc2_core_media::CMSampleBuffer;
use objc2_foundation::{NSDate, NSDictionary, NSError, NSNumber, NSRunLoop, NSString, NSURL};

use block2::RcBlock;
use std::sync::mpsc;

use crate::cli::ContainerFormat;
use crate::error::CliError;

const AUDIO_FORMAT_ID_MPEG4_AAC: u32 = 0x6161_6320;
const SYSTEM_AUDIO_SAMPLE_RATE_HZ: f64 = 48_000.0;
const SYSTEM_AUDIO_CHANNEL_COUNT: i32 = 2;
const MIC_AUDIO_SAMPLE_RATE_HZ: f64 = 48_000.0;
const MIC_AUDIO_CHANNEL_COUNT: i32 = 1;

#[derive(Debug, Clone, Copy, Default)]
pub struct AudioConfig {
    pub system: bool,
    pub mic: bool,
}

pub struct AssetWriter {
    writer: Retained<AVAssetWriter>,
    video_input: Retained<AVAssetWriterInput>,
    system_audio_input: Option<Retained<AVAssetWriterInput>>,
    mic_audio_input: Option<Retained<AVAssetWriterInput>>,
    started: bool,
}

impl AssetWriter {
    pub fn new(
        path: &Path,
        format: ContainerFormat,
        width: i32,
        height: i32,
        audio: AudioConfig,
    ) -> Result<Self, CliError> {
        if width <= 0 || height <= 0 {
            return Err(CliError::runtime("invalid capture dimensions"));
        }

        if path.exists() {
            std::fs::remove_file(path)
                .map_err(|err| CliError::runtime(format!("failed to remove output file: {err}")))?;
        }

        let file_type = unsafe {
            match format {
                ContainerFormat::Mov => AVFileTypeQuickTimeMovie,
                ContainerFormat::Mp4 => AVFileTypeMPEG4,
            }
        }
        .ok_or_else(|| CliError::runtime("missing AVFoundation file type"))?;

        let path_string = path.to_string_lossy();
        let ns_path = NSString::from_str(&path_string);
        let url = NSURL::fileURLWithPath(&ns_path);
        let writer = unsafe { AVAssetWriter::assetWriterWithURL_fileType_error(&url, file_type) }
            .map_err(|err| ns_error_to_cli("failed to create asset writer", &err))?;

        let media_type = unsafe { AVMediaTypeVideo }
            .ok_or_else(|| CliError::runtime("missing AVFoundation media type"))?;
        let codec_key = unsafe { AVVideoCodecKey }
            .ok_or_else(|| CliError::runtime("missing AVVideoCodecKey"))?;
        let width_key = unsafe { AVVideoWidthKey }
            .ok_or_else(|| CliError::runtime("missing AVVideoWidthKey"))?;
        let height_key = unsafe { AVVideoHeightKey }
            .ok_or_else(|| CliError::runtime("missing AVVideoHeightKey"))?;
        let codec = unsafe { AVVideoCodecTypeH264 }
            .ok_or_else(|| CliError::runtime("missing H.264 codec"))?;

        let width_value = NSNumber::numberWithInt(width);
        let height_value = NSNumber::numberWithInt(height);
        let keys: [&NSString; 3] = [codec_key, width_key, height_key];
        let values: [&AnyObject; 3] = [codec, &*width_value, &*height_value];
        let settings = NSDictionary::from_slices(&keys, &values);

        let input = unsafe {
            AVAssetWriterInput::assetWriterInputWithMediaType_outputSettings(
                media_type,
                Some(&settings),
            )
        };
        unsafe { input.setExpectsMediaDataInRealTime(true) };
        unsafe { writer.addInput(&input) };

        let audio_media_type = unsafe { AVMediaTypeAudio }
            .ok_or_else(|| CliError::runtime("missing AVFoundation audio media type"))?;

        let system_audio_input = if audio.system {
            let audio_settings =
                audio_settings(SYSTEM_AUDIO_SAMPLE_RATE_HZ, SYSTEM_AUDIO_CHANNEL_COUNT);
            let input = unsafe {
                AVAssetWriterInput::assetWriterInputWithMediaType_outputSettings(
                    audio_media_type,
                    Some(&audio_settings),
                )
            };
            unsafe { input.setExpectsMediaDataInRealTime(true) };
            unsafe { writer.addInput(&input) };
            Some(input)
        } else {
            None
        };

        let mic_audio_input = if audio.mic {
            let audio_settings = audio_settings(MIC_AUDIO_SAMPLE_RATE_HZ, MIC_AUDIO_CHANNEL_COUNT);
            let input = unsafe {
                AVAssetWriterInput::assetWriterInputWithMediaType_outputSettings(
                    audio_media_type,
                    Some(&audio_settings),
                )
            };
            unsafe { input.setExpectsMediaDataInRealTime(true) };
            unsafe { writer.addInput(&input) };
            Some(input)
        } else {
            None
        };

        Ok(Self {
            writer,
            video_input: input,
            system_audio_input,
            mic_audio_input,
            started: false,
        })
    }

    pub fn append_sample_buffer(&mut self, sample_buffer: &CMSampleBuffer) -> Result<(), CliError> {
        self.append_video_sample_buffer(sample_buffer)
    }

    pub fn append_video_sample_buffer(
        &mut self,
        sample_buffer: &CMSampleBuffer,
    ) -> Result<(), CliError> {
        self.ensure_started(sample_buffer)?;
        Self::append_to_input(&self.writer, &self.video_input, sample_buffer)
    }

    pub fn append_system_audio_sample_buffer(
        &mut self,
        sample_buffer: &CMSampleBuffer,
    ) -> Result<(), CliError> {
        if self.system_audio_input.is_none() {
            return Ok(());
        }
        self.ensure_started(sample_buffer)?;
        let input = self
            .system_audio_input
            .as_ref()
            .ok_or_else(|| CliError::runtime("missing system audio input"))?;
        Self::append_to_input(&self.writer, input, sample_buffer)
    }

    pub fn append_mic_audio_sample_buffer(
        &mut self,
        sample_buffer: &CMSampleBuffer,
    ) -> Result<(), CliError> {
        if self.mic_audio_input.is_none() {
            return Ok(());
        }
        self.ensure_started(sample_buffer)?;
        let input = self
            .mic_audio_input
            .as_ref()
            .ok_or_else(|| CliError::runtime("missing microphone audio input"))?;
        Self::append_to_input(&self.writer, input, sample_buffer)
    }

    fn append_to_input(
        writer: &AVAssetWriter,
        input: &AVAssetWriterInput,
        sample_buffer: &CMSampleBuffer,
    ) -> Result<(), CliError> {
        let ready = unsafe { input.isReadyForMoreMediaData() };
        if !ready {
            return Ok(());
        }

        let appended = unsafe { input.appendSampleBuffer(sample_buffer) };
        if !appended {
            return Err(writer_status_error(
                "failed to append sample buffer",
                writer,
            ));
        }
        Ok(())
    }

    fn ensure_started(&mut self, sample_buffer: &CMSampleBuffer) -> Result<(), CliError> {
        if self.started {
            return Ok(());
        }

        let ok = unsafe { self.writer.startWriting() };
        if !ok {
            return Err(writer_status_error(
                "failed to start asset writer",
                &self.writer,
            ));
        }
        let start_time = unsafe { sample_buffer.presentation_time_stamp() };
        unsafe { self.writer.startSessionAtSourceTime(start_time) };
        self.started = true;
        Ok(())
    }

    pub fn finish(self) -> Result<(), CliError> {
        if !self.started {
            return Err(CliError::runtime("no frames captured"));
        }

        unsafe { self.video_input.markAsFinished() };
        if let Some(input) = self.system_audio_input.as_ref() {
            unsafe { input.markAsFinished() };
        }
        if let Some(input) = self.mic_audio_input.as_ref() {
            unsafe { input.markAsFinished() };
        }

        let (sender, receiver) = mpsc::channel();
        let block = RcBlock::new(move || {
            let _ = sender.send(());
        });

        unsafe { self.writer.finishWritingWithCompletionHandler(&block) };
        wait_for_completion(&receiver, "finish writing")?;

        if unsafe { self.writer.status() } != objc2_av_foundation::AVAssetWriterStatus::Completed {
            return Err(writer_status_error("asset writer failed", &self.writer));
        }

        Ok(())
    }
}

pub fn write_diagnostics_contact_sheet(
    path: &Path,
    source_output_path: &Path,
    duration_ms: u64,
) -> Result<(), CliError> {
    let source_name = source_output_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("recording");
    let escaped_name = escape_svg(source_name);
    let view_duration = duration_ms.max(1);

    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"960\" height=\"540\" viewBox=\"0 0 960 540\">
  <defs>
    <linearGradient id=\"bg\" x1=\"0\" y1=\"0\" x2=\"1\" y2=\"1\">
      <stop offset=\"0%\" stop-color=\"#0d1b2a\" />
      <stop offset=\"100%\" stop-color=\"#1b263b\" />
    </linearGradient>
  </defs>
  <rect x=\"0\" y=\"0\" width=\"960\" height=\"540\" fill=\"url(#bg)\" />
  <rect x=\"40\" y=\"80\" width=\"400\" height=\"220\" fill=\"#415a77\" rx=\"12\" />
  <rect x=\"520\" y=\"80\" width=\"400\" height=\"220\" fill=\"#415a77\" rx=\"12\" />
  <rect x=\"40\" y=\"320\" width=\"400\" height=\"180\" fill=\"#778da9\" rx=\"12\" />
  <rect x=\"520\" y=\"320\" width=\"400\" height=\"180\" fill=\"#778da9\" rx=\"12\" />
  <text x=\"48\" y=\"42\" fill=\"#e0e1dd\" font-size=\"28\" font-family=\"Menlo, monospace\">screen-record diagnostics</text>
  <text x=\"48\" y=\"508\" fill=\"#e0e1dd\" font-size=\"18\" font-family=\"Menlo, monospace\">source={escaped_name} duration_ms={view_duration}</text>
</svg>
"
    );
    std::fs::write(path, svg)
        .map_err(|err| CliError::runtime(format!("failed to write contact sheet: {err}")))
}

pub fn write_diagnostics_motion_intervals(
    path: &Path,
    source_output_path: &Path,
    duration_ms: u64,
) -> Result<usize, CliError> {
    let source_path = source_output_path.display().to_string();
    let escaped_source_path = escape_json(&source_path);
    let span = duration_ms.max(1);
    let text = format!(
        "{{\n  \"schema_version\": 1,\n  \"contract_version\": \"1.0\",\n  \"source_output_path\": \"{}\",\n  \"intervals\": [\n    {{ \"start_ms\": 0, \"end_ms\": {}, \"score\": 100 }}\n  ]\n}}\n",
        escaped_source_path, span
    );
    std::fs::write(path, text)
        .map_err(|err| CliError::runtime(format!("failed to write motion intervals: {err}")))?;
    Ok(1)
}

fn audio_settings(
    sample_rate_hz: f64,
    channel_count: i32,
) -> Retained<NSDictionary<NSString, AnyObject>> {
    let format_key = NSString::from_str("AVFormatIDKey");
    let sample_rate_key = NSString::from_str("AVSampleRateKey");
    let channels_key = NSString::from_str("AVNumberOfChannelsKey");

    let format_value = NSNumber::numberWithUnsignedInt(AUDIO_FORMAT_ID_MPEG4_AAC);
    let sample_rate_value = NSNumber::numberWithDouble(sample_rate_hz);
    let channel_value = NSNumber::numberWithInt(channel_count);

    let keys: [&NSString; 3] = [&*format_key, &*sample_rate_key, &*channels_key];
    let values: [&AnyObject; 3] = [&*format_value, &*sample_rate_value, &*channel_value];
    NSDictionary::from_slices(&keys, &values)
}

fn writer_status_error(prefix: &str, writer: &AVAssetWriter) -> CliError {
    let error = unsafe { writer.error() };
    match error {
        Some(err) => ns_error_to_cli(prefix, &err),
        None => CliError::runtime(prefix),
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

fn wait_for_completion(receiver: &mpsc::Receiver<()>, label: &str) -> Result<(), CliError> {
    let run_loop = NSRunLoop::currentRunLoop();
    loop {
        match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(()) => return Ok(()),
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

fn escape_json(value: &str) -> String {
    let mut escaped = String::new();
    for ch in value.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            '\u{08}' => escaped.push_str("\\b"),
            '\u{0C}' => escaped.push_str("\\f"),
            ch if ch <= '\u{1F}' => escaped.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => escaped.push(ch),
        }
    }
    escaped
}

fn escape_svg(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            '&' => "&amp;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '"' => "&quot;".to_string(),
            '\'' => "&apos;".to_string(),
            _ => ch.to_string(),
        })
        .collect()
}
