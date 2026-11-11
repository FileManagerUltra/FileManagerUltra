use std::sync::mpsc::{Receiver, TryRecvError, sync_channel};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use fm_vfs::FmError;

use crate::{AudioConsumer, AudioFormat, AudioProducer, Clock, Gain, audio_buffer};

/// Output device abstraction. Implementations and their streams stay on the playback worker.
pub trait AudioSink {
    fn format(&self) -> AudioFormat;
    fn play(&self) -> Result<(), FmError>;
    fn pause(&self) -> Result<(), FmError>;
    fn check_error(&self) -> Result<(), FmError>;
}

pub struct DeviceSink {
    stream: cpal::Stream,
    format: AudioFormat,
    errors: Receiver<cpal::Error>,
}

impl DeviceSink {
    /// Discover and open the default device. Never call this from the UI thread.
    pub fn open(gain: Gain) -> Result<(Self, AudioProducer, Clock), FmError> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or_else(|| {
            crate::error("No audio output device. Connect one and try Play again.")
        })?;
        let config = device
            .default_output_config()
            .map_err(|e| crate::error(format!("Cannot read audio device configuration: {e}")))?;
        let format = AudioFormat::new(config.sample_rate(), usize::from(config.channels()))?;
        let (writer, consumer, clock) = audio_buffer(format, format.rate() as usize / 2, gain)?;
        let (errors_tx, errors) = sync_channel(1);
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => {
                build::<f32>(&device, config.config(), consumer, clock.clone(), errors_tx)
            }
            cpal::SampleFormat::F64 => {
                build::<f64>(&device, config.config(), consumer, clock.clone(), errors_tx)
            }
            cpal::SampleFormat::I8 => {
                build::<i8>(&device, config.config(), consumer, clock.clone(), errors_tx)
            }
            cpal::SampleFormat::I16 => {
                build::<i16>(&device, config.config(), consumer, clock.clone(), errors_tx)
            }
            cpal::SampleFormat::I32 => {
                build::<i32>(&device, config.config(), consumer, clock.clone(), errors_tx)
            }
            cpal::SampleFormat::I64 => {
                build::<i64>(&device, config.config(), consumer, clock.clone(), errors_tx)
            }
            cpal::SampleFormat::U8 => {
                build::<u8>(&device, config.config(), consumer, clock.clone(), errors_tx)
            }
            cpal::SampleFormat::U16 => {
                build::<u16>(&device, config.config(), consumer, clock.clone(), errors_tx)
            }
            cpal::SampleFormat::U32 => {
                build::<u32>(&device, config.config(), consumer, clock.clone(), errors_tx)
            }
            cpal::SampleFormat::U64 => {
                build::<u64>(&device, config.config(), consumer, clock.clone(), errors_tx)
            }
            other => {
                return Err(crate::error(format!(
                    "Unsupported audio device format: {other}"
                )));
            }
        }
        .map_err(|e| crate::error(format!("Cannot open audio output: {e}")))?;
        Ok((
            Self {
                stream,
                format,
                errors,
            },
            writer,
            clock,
        ))
    }
}

fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut consumer: AudioConsumer,
    clock: Clock,
    errors: std::sync::mpsc::SyncSender<cpal::Error>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    device.build_output_stream(
        config,
        move |output: &mut [T], info| {
            let timestamp = info.timestamp();
            clock.set_latency(timestamp.playback.duration_since(timestamp.callback));
            consumer.render(output);
        },
        move |err| {
            // The first outstanding device failure is enough to stop playback; glitches are not.
            if is_fatal(&err) {
                let _ = errors.try_send(err);
            }
        },
        None,
    )
}

/// Underruns, refused real-time scheduling and automatic re-routing leave the stream playing
/// (at worst with a click), so they must not end playback.
fn is_fatal(err: &cpal::Error) -> bool {
    !matches!(
        err.kind(),
        cpal::ErrorKind::Xrun | cpal::ErrorKind::RealtimeDenied | cpal::ErrorKind::DeviceChanged
    )
}

impl AudioSink for DeviceSink {
    fn format(&self) -> AudioFormat {
        self.format
    }

    fn play(&self) -> Result<(), FmError> {
        self.stream
            .play()
            .map_err(|e| crate::error(format!("Cannot start audio output: {e}")))
    }

    fn pause(&self) -> Result<(), FmError> {
        self.stream
            .pause()
            .map_err(|e| crate::error(format!("Cannot pause audio output: {e}")))
    }

    fn check_error(&self) -> Result<(), FmError> {
        match self.errors.try_recv() {
            Ok(err) => Err(crate::error(format!(
                "Audio device failed. Check the output device and retry: {err}"
            ))),
            Err(TryRecvError::Empty) => Ok(()),
            Err(TryRecvError::Disconnected) => Err(crate::error("Audio output disconnected")),
        }
    }
}

#[cfg(test)]
mod fatal_tests {
    use super::is_fatal;
    use cpal::{Error, ErrorKind};

    #[test]
    fn glitches_do_not_stop_playback_but_lost_devices_do() {
        for kind in [
            ErrorKind::Xrun,
            ErrorKind::RealtimeDenied,
            ErrorKind::DeviceChanged,
        ] {
            assert!(!is_fatal(&Error::new(kind)), "{kind:?}");
        }
        for kind in [ErrorKind::DeviceNotAvailable, ErrorKind::BackendError] {
            assert!(is_fatal(&Error::new(kind)), "{kind:?}");
        }
    }
}
