pub mod beats;
pub mod clock;
pub mod dsp;
pub mod mixer;
pub mod multicam;
pub mod output;
pub mod recorder;
pub mod spatial_master;
pub mod waveform;

pub use beats::{
    BeatAnalysisResult, BeatDetectionConfig, BeatPoint, detect_audio_beats,
    detect_beats_from_waveform,
};
pub use clock::AudioMasterClock;
pub use dsp::{
    AtomicAudioMeter, AudioMeterSnapshot, SpectrumSnapshot, StemSeparationResult,
    analyze_spectrum_bands, apply_audio_effect_chain, apply_de_esser, apply_sidechain_ducking,
    apply_spectral_noise_reduction, compute_lufs_gain, db_to_linear, linear_to_db,
    measure_block_meter, measure_integrated_lufs, separate_vocal_bgm_stems,
};
pub use mixer::{AudioClip, AudioGraph};
pub use multicam::{AudioSyncEstimate, estimate_audio_offset_xcorr};
pub use output::{AudioEngineState, AudioOutputError};
pub use recorder::{VoiceoverError, VoiceoverRecorder};
pub use spatial_master::{
    apply_binaural_spatial_3d, apply_multiband_compressor_3band, apply_true_peak_limiter,
    measure_true_peak_dbtp,
};
pub use waveform::{WaveformBucket, WaveformPeaks};
