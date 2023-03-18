use anyhow::Result;
use byteorder::{LittleEndian, ReadBytesExt};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::{HeapProducer, HeapRb};
use rubato::{FftFixedInOut, Resampler};
use std::error::Error;

const SAMPLES_HIGH: i32 = 752;

#[derive(Debug)]
pub enum AudioPlayerError {
    NoDefaultOutputDevice,
    DualChannelNotSupported,
}

impl Error for AudioPlayerError {}

impl std::fmt::Display for AudioPlayerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoDefaultOutputDevice => write!(f, "No default output device"),
            Self::DualChannelNotSupported => write!(f, "Dual channel not supported"),
        }
    }
}

pub struct AudioPlayer {
    buffer_producer: HeapProducer<f32>,
    resampler: Option<FftFixedInOut<f32>>,
    pre_resampled_buffer: Vec<f32>,
    pre_resampled_split_buffers: [Vec<f32>; 2],
    resample_process_buffers: [Vec<f32>; 2],
    resampled_buffer: Vec<f32>,
    output_stream: cpal::Stream,
}

impl AudioPlayer {
    pub fn new(sample_rate: u32, channels: u16) -> Result<Self> {
        let host = cpal::default_host();
        let output_device = host.default_output_device();

        let output_device = match output_device {
            Some(device) => device,
            None => return Err(AudioPlayerError::NoDefaultOutputDevice.into()),
        };

        let sample_rate = cpal::SampleRate(sample_rate);

        let conf = output_device
            .supported_output_configs()?
            .collect::<Vec<_>>();

        let mut found_conf = false;

        for c in &conf {
            // must have 2 channels and f32 format
            // (almost all? devices will have at least one configuration with these)
            if c.channels() == 2
                && c.sample_format() == cpal::SampleFormat::F32
                && c.min_sample_rate() <= sample_rate
                && c.max_sample_rate() >= sample_rate
            {
                found_conf = true;
                break;
            }
        }

        let (output_sample_rate, resampler) = if found_conf {
            (sample_rate, None)
        } else {
            let def_conf = output_device.default_output_config().unwrap();

            if def_conf.channels() != 2 || def_conf.sample_format() != cpal::SampleFormat::F32 {
                eprintln!("[Audio] No supported configuration found for audio device, please open an issue in github `dcvz/helix`\n\
                      list of supported configurations: {conf:#?}");
                return Err(AudioPlayerError::DualChannelNotSupported.into());
            }

            (
                def_conf.sample_rate(),
                Some(FftFixedInOut::<f32>::new(
                    sample_rate.0 as usize,
                    def_conf.sample_rate().0 as usize,
                    sample_rate.0 as usize / 60,
                    2,
                )?),
            )
        };

        let config = cpal::StreamConfig {
            channels,
            sample_rate: output_sample_rate,
            buffer_size: cpal::BufferSize::Default,
        };

        // Limiting the number of samples in the buffer is better to minimize
        // audio delay in playback, this is because game speed
        // does not 100% match audio playing speed (44100Hz).
        // The buffer holds only audio for 1/4 second, which is good enough for delays,
        // It can be reduced more, but it might cause noise(?) for slower machines
        // or if any CPU intensive process started while the emulator is running
        let buffer = HeapRb::new(output_sample_rate.0 as usize / 2);
        let (buffer_producer, mut buffer_consumer) = buffer.split();

        let output_data_fn = move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            for sample in data {
                *sample = buffer_consumer.pop().unwrap_or(0.);
            }
        };

        let output_stream =
            output_device.build_output_stream(&config, output_data_fn, Self::err_fn, None)?;

        output_stream.play().unwrap();

        Ok(Self {
            buffer_producer,
            output_stream,
            pre_resampled_buffer: Vec::new(),
            pre_resampled_split_buffers: [Vec::new(), Vec::new()],
            resample_process_buffers: [Vec::new(), Vec::new()],
            resampled_buffer: Vec::new(),
            resampler,
        })
    }

    pub fn buffered(&self) -> i32 {
        self.buffer_producer.len() as i32 / 4
    }

    pub fn desired_buffer(&self) -> i32 {
        SAMPLES_HIGH * 4
    }

    pub fn play_buffer(&mut self, buf: &[u8]) {
        // helper method to split channels into separate vectors
        fn read_frames(inbuffer: &[f32], n_frames: usize, outputs: &mut [Vec<f32>]) {
            for output in outputs.iter_mut() {
                output.clear();
                output.reserve(n_frames);
            }
            let mut value: f32;
            let mut inbuffer_iter = inbuffer.iter();
            for _ in 0..n_frames {
                for output in outputs.iter_mut() {
                    value = *inbuffer_iter.next().unwrap();
                    output.push(value);
                }
            }
        }

        /// Helper to merge channels into a single vector
        fn write_frames(waves: &[Vec<f32>], outbuffer: &mut Vec<f32>) {
            let nbr = waves[0].len();
            for frame in 0..nbr {
                for wave in waves.iter() {
                    outbuffer.push(wave[frame]);
                }
            }
        }

        // transform the buffer into a vector of f32 samples
        // buffer data is of 2 channels, 16 bit samples
        let mut cursor = std::io::Cursor::new(buf);
        let mut samples = Vec::with_capacity(buf.len() / 2);
        while let Ok(sample) = cursor.read_i16::<LittleEndian>() {
            samples.push(sample as f32 / 32768.0);
        }

        if let Some(resampler) = &mut self.resampler {
            self.pre_resampled_buffer.extend_from_slice(&samples);

            loop {
                let frames = resampler.input_frames_next();
                if self.pre_resampled_buffer.len() < frames * 2 {
                    return;
                }

                // only read the needed frames
                read_frames(
                    &self.pre_resampled_buffer,
                    frames,
                    &mut self.pre_resampled_split_buffers,
                );

                self.resample_process_buffers[0].clear();
                self.resample_process_buffers[0].clear();

                let output_frames = resampler.output_frames_next();
                self.resample_process_buffers[0].reserve(output_frames);
                self.resample_process_buffers[1].reserve(output_frames);

                resampler
                    .process_into_buffer(
                        &self.pre_resampled_split_buffers,
                        &mut self.resample_process_buffers,
                        None,
                    )
                    .unwrap();

                // resample
                if self.resampled_buffer.len() < output_frames * 2 {
                    self.resampled_buffer
                        .reserve(output_frames * 2 - self.resampled_buffer.len());
                }
                self.resampled_buffer.clear();
                write_frames(&self.resample_process_buffers, &mut self.resampled_buffer);

                self.buffer_producer.push_slice(&self.resampled_buffer);

                self.pre_resampled_buffer = self.pre_resampled_buffer.split_off(frames * 2);
            }
        } else {
            self.buffer_producer.push_slice(&samples);
        }
    }

    fn err_fn(err: cpal::StreamError) {
        eprintln!("[Audio] an error occurred on audio stream: {err}");
    }
}

// MARK: - C API

#[cfg(feature = "cpp")]
#[no_mangle]
pub extern "C" fn HLXAudioPlayerCreate(sample_rate: u32, channels: u16) -> Box<AudioPlayer> {
    match AudioPlayer::new(sample_rate, channels) {
        Ok(player) => Box::new(player),
        Err(err) => {
            eprintln!("[Audio] failed to create audio player: {err}");
            unsafe { Box::from_raw(std::ptr::null_mut()) }
        }
    }
}

#[cfg(feature = "cpp")]
#[no_mangle]
pub extern "C" fn HLXAudioPlayerFree(player: Option<Box<AudioPlayer>>) {
    if let Some(player) = player {
        drop(player);
    }
}

#[cfg(feature = "cpp")]
#[no_mangle]
pub extern "C" fn HLXAudioPlayerGetBuffered(player: Option<&mut AudioPlayer>) -> i32 {
    match player {
        Some(player) => player.buffered(),
        None => {
            eprintln!(
                "[Audio] failed to get buffered audio: was given an invalid instance pointer"
            );
            0
        }
    }
}

#[cfg(feature = "cpp")]
#[no_mangle]
pub extern "C" fn HLXAudioPlayerGetDesiredBuffered(player: Option<&mut AudioPlayer>) -> i32 {
    match player {
        Some(player) => player.desired_buffer(),
        None => {
            eprintln!("[Audio] failed to get desired buffered audio: was given an invalid instance pointer");
            0
        }
    }
}

#[cfg(feature = "cpp")]
#[no_mangle]
pub extern "C" fn HLXAudioPlayerPlayBuffer(
    player: Option<&mut AudioPlayer>,
    buf: *const u8,
    len: usize,
) {
    match player {
        Some(player) => {
            let buf = unsafe { std::slice::from_raw_parts(buf, len) };
            player.play_buffer(buf);
        }
        None => {
            eprintln!("[Audio] failed to play audio buffer: was given an invalid instance pointer")
        }
    }
}
