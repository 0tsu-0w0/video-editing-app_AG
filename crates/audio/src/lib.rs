pub mod beats;
pub mod clock;
pub mod dsp;
pub mod mixer;
pub mod output;
pub mod waveform;

pub use beats::{
    BeatAnalysisResult, BeatDetectionConfig, BeatPoint, detect_audio_beats,
    detect_beats_from_waveform,
};
pub use clock::AudioMasterClock;
pub use dsp::{
    AtomicAudioMeter, AudioMeterSnapshot, SpectrumSnapshot, analyze_spectrum_bands,
    apply_audio_effect_chain, apply_sidechain_ducking, compute_lufs_gain, db_to_linear,
    linear_to_db, measure_block_meter, measure_integrated_lufs,
};
pub use mixer::{AudioClip, AudioGraph};
pub use output::{AudioEngineState, AudioOutputError};
pub use waveform::{WaveformBucket, WaveformPeaks};
