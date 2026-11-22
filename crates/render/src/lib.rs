pub mod ai_vision;
pub mod color;
pub mod color_grading;
pub mod graph;
pub mod hot_shader;
pub mod mask_tracker;
pub mod node_eval;
pub mod pipeline;
pub mod psd_sprite;
pub mod sprite;
pub mod text;
pub mod v09_fx;
pub mod vfx_track;

pub use ai_vision::{
    AutoReframeConfig, LocalVisionBackend, LocalVisionStatus, MotionVector, OpticalFlowConfig,
    ReframeSample, SegmentationConfig, SuperResolutionConfig, analyze_scene_auto_reframe,
    apply_ai_segmentation, apply_ai_super_res_detail, build_auto_reframe_commands,
    detect_local_vision_status, detect_subject_focus_in_frame, estimate_optical_flow,
    interpolate_frame_optical_flow, smooth_reframe_trajectory, upscale_super_resolution,
};
pub use color::{LinearImage, LinearPremul, linear_to_srgb, srgb_to_linear};
pub use color_grading::{
    BUILTIN_LUT_PRESETS, CubeLut3D, HIST_BINS, PARADE_BINS, PARADE_COLS, VECTORSCOPE_SIZE,
    VideoScopeData, WAVEFORM_BINS, WAVEFORM_COLS, apply_hsl_qualifier_rgb, apply_scene_color_grade,
    compute_auto_color_match, compute_video_scopes, emit_sample_cube_luts, evaluate_tone_curve_lut,
};
pub use graph::{EFFECTS_WGSL_SOURCE, RenderGraph, render_frame_batch_parallel};
pub use hot_shader::{
    HotShaderEntry, ShaderHotReloader, ShaderReloadStatus, register_shader_dir, validate_wgsl,
};
pub use mask_tracker::{apply_bezier_masks, track_point_across_frames, track_point_in_scene};
pub use node_eval::{LayerRenderPacket, evaluate_node_composite_graph};
pub use pipeline::{
    COMPOSITE_COMPUTE_WGSL_SOURCE, GpuEffectUniform, GpuPostProcessPass, HeadlessGpuOffloader,
    WgpuComputeCompositor, WgpuPreviewPipeline,
};
pub use psd_sprite::PsdCharacterAsset;
pub use sprite::{load_png_linear, render_placeholder_character, scale_linear_image};
pub use text::{StyledSpan, TextRenderer, parse_inline_text_spans, strip_inline_tags};
pub use v09_fx::{
    apply_3d_lighting_tint, build_drop_shadow_image, render_particle_emitter, render_vector_shape,
};
pub use vfx_track::{
    CameraMotionSample, StabilizeConfig, analyze_camera_shake, apply_corner_pin,
    build_stabilize_commands, compute_shake_compensation, estimate_global_frame_motion, invert_3x3,
    solve_unit_square_to_quad_homography,
};
