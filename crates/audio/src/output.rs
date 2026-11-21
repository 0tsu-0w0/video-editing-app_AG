use crate::clock::AudioMasterClock;
use crate::dsp::AtomicAudioMeter;
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
    pub meter: AtomicAudioMeter,
}

impl AudioEngineState {
    pub fn new(sample_rate: u32) -> Arc<Self> {
        Arc::new(Self {
            graph: RwLock::new(AudioGraph::new(sample_rate)),
            clock: RwLock::new(AudioMasterClock::new(sample_rate)),
            meter: AtomicAudioMeter::new(),
        })
    }
}

#[cfg(any(windows, feature = "cpal-output"))]
pub mod live {
    use super::*;
    use crate::dsp::{AudioMeterSnapshot, measure_block_meter};
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
        // リアルタイム音声スレッドでの毎コールバック Heap 確保を避けるためスクラッチバッファを事前確保
        let mut stereo_scratch = vec![0.0f32; 4096];

        let stream = device
            .build_output_stream(
                &config.into(),
                move |data: &mut [f32], _| {
                    let ch_count = channels.max(1);
                    let frames = data.len() / ch_count;
                    let (playing, start_sample) = match state.clock.try_read() {
                        Ok(clock) => (clock.is_playing(), clock.current_sample()),
                        Err(_) => {
                            data.fill(0.0);
                            return;
                        }
                    };
                    if !playing {
                        data.fill(0.0);
                        state.meter.store(AudioMeterSnapshot::default());
                        return;
                    }
                    let needed = frames * 2;
                    if stereo_scratch.len() < needed {
                        stereo_scratch.resize(needed, 0.0);
                    }
                    let stereo = &mut stereo_scratch[..needed];
                    if let Ok(graph) = state.graph.try_read() {
                        graph.render_block(start_sample, frames, stereo);
                    } else {
                        stereo.fill(0.0);
                    }
                    state.meter.store(measure_block_meter(stereo));
                    for i in 0..frames {
                        for ch in 0..ch_count {
                            data[i * ch_count + ch] = stereo[i * 2 + ch.min(1)];
                        }
                    }
                    if let Ok(clock) = state.clock.try_read() {
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
