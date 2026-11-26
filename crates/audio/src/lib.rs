pub mod beats;
pub mod bgm_composer;
pub mod clock;
pub mod dsp;
pub mod dubbing;
pub mod foley;
pub mod mixer;
pub mod multicam;
pub mod output;
pub mod podcast_studio;
pub mod recorder;
pub mod restoration;
pub mod spatial_master;
pub mod waveform;

pub use beats::{
    BeatAnalysisResult, BeatDetectionConfig, BeatPoint, detect_audio_beats,
    detect_beats_from_waveform,
};
pub use bgm_composer::{BgmComposeConfig, BgmGenre, ComposedBgmResult, compose_bgm_track};
pub use clock::AudioMasterClock;
pub use dsp::{
    AtomicAudioMeter, AudioMeterSnapshot, SpectrumSnapshot, StemSeparationResult,
    analyze_spectrum_bands, apply_audio_effect_chain, apply_de_esser, apply_sidechain_ducking,
    apply_spectral_noise_reduction, compute_lufs_gain, db_to_linear, linear_to_db,
    measure_block_meter, measure_integrated_lufs, separate_vocal_bgm_stems,
};
pub use dubbing::{
    DubbingVoicePreset, GeneratedDubClip, align_text_to_phoneme_segments,
    build_dubbing_timeline_commands, collect_dubbing_targets_from_table,
    synthesize_offline_dubbing_pcm,
};
pub use foley::{
    FoleyPreset, encode_stereo_wav_pcm16, synthesize_foley_sfx_pcm, write_stereo_wav_file,
};
pub use mixer::{AudioClip, AudioGraph};
pub use multicam::{AudioSyncEstimate, estimate_audio_offset_xcorr};
pub use output::{AudioEngineState, AudioOutputError};
pub use podcast_studio::{
    PodcastAnalysisResult, SpeakerTrackSpec, SpeakerTurnSegment, analyze_podcast_speakers,
    apply_podcast_voice_leveler, build_podcast_auto_switch_commands,
};
pub use recorder::{VoiceoverError, VoiceoverRecorder};
pub use restoration::{
    DeclickReport, FoaAmbiXBuffer, Surround51Buffer, apply_adaptive_dereverb, apply_declick,
    apply_hum_removal, build_adr_cue_timeline_commands, decode_foa_ambix_to_binaural_stereo,
    downmix_51_to_stereo_itu, encode_stereo_to_51_surround, encode_stereo_to_foa_ambix,
    rotate_foa_ambix, synthesize_adr_preroll_beeps, write_multichannel_wav_file,
};
pub use spatial_master::{
    apply_binaural_spatial_3d, apply_multiband_compressor_3band, apply_true_peak_limiter,
    measure_true_peak_dbtp,
};
pub use waveform::{WaveformBucket, WaveformPeaks};
