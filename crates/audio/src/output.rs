use crate::clock::AudioMasterClock;
use crate::mixer::AudioGraph;
use std::sync::{Arc, RwLock};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AudioOutputError {
    #[error("No audio output device available")]
    NoDevice,
    #[error("Audio backend error: {0}")]
    Backend(String),
}

pub struct AudioEngineState {
    pub graph: RwLock<AudioGraph>,
    pub clock: RwLock<AudioMasterClock>,
}

impl AudioEngineState {
    pub fn new(sample_rate: u32) -> Arc<Self> {
        Arc::new(Self {
            graph: RwLock::new(AudioGraph::new(sample_rate)),
            clock: RwLock::new(AudioMasterClock::new(sample_rate)),
        })
    }
}

#[cfg(any(windows, feature = "cpal-output"))]
pub mod live {
    use super::*;
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    pub struct LiveAudioStream {
        _stream: cpal::Stream,
    }

    pub fn start_live_output(
        state: Arc<AudioEngineState>,
    ) -> Result<LiveAudioStream, AudioOutputError> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or(AudioOutputError::NoDevice)?;
        let config = device
            .default_output_config()
            .map_err(|e| AudioOutputError::Backend(e.to_string()))?;
        let channels = config.channels() as usize;

        let stream = device
            .build_output_stream(
                &config.into(),
                move |data: &mut [f32], _| {
                    let frames = data.len() / channels.max(1);
                    let (playing, start_sample) = {
                        let clock = state.clock.read().unwrap();
                        (clock.is_playing(), clock.current_sample())
                    };
                    if !playing {
                        data.fill(0.0);
                        return;
                    }
                    let mut stereo = vec![0.0f32; frames * 2];
                    if let Ok(graph) = state.graph.read() {
                        graph.render_block(start_sample, frames, &mut stereo);
                    }
                    for i in 0..frames {
                        for ch in 0..channels {
                            data[i * channels + ch] = stereo[i * 2 + ch.min(1)];
                        }
                    }
                    if let Ok(clock) = state.clock.read() {
                        clock.advance_by_audio_callback(frames);
                    }
                },
                |err| eprintln!("cpal output stream error: {err}"),
                None,
            )
            .map_err(|e| AudioOutputError::Backend(e.to_string()))?;

        stream
            .play()
            .map_err(|e| AudioOutputError::Backend(e.to_string()))?;
        Ok(LiveAudioStream { _stream: stream })
    }
}
