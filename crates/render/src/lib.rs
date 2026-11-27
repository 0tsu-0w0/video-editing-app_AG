pub mod ai_vision;
pub mod color;
pub mod color_grading;
pub mod finishing_render;
pub mod generative_render;
pub mod gpu_v100;
pub mod gpu_v30;
pub mod gpu_v70;
pub mod graph;
pub mod hot_shader;
pub mod mask_tracker;
pub mod mesh3d_render;
pub mod node_eval;
pub mod pipeline;
pub mod psd_sprite;
pub mod rig_render;
pub mod sprite;
pub mod text;
pub mod v09_fx;
pub mod v100_render;
pub mod v60_render;
pub mod v80_render;
pub mod v90_render;
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
    apply_scene_color_management, compute_auto_color_match, compute_video_scopes,
    emit_sample_cube_luts, evaluate_tone_curve_lut,
};
pub use finishing_render::apply_cinema_finishing;
pub use generative_render::{
    encode_linear_image_png_bytes, measure_scene_video_qc_metrics, render_generative_bg,
    score_frame_thumbnail_attractiveness, select_best_thumbnail_frame,
};
pub use gpu_v30::{
    COLOR_GRADE_COMPUTE_WGSL, GpuBenchmarkReport, GpuV30Capabilities, GpuV30Pipeline,
    VIDEO_SCOPES_COMPUTE_WGSL, benchmark_v30_gpu_pipeline, detect_gpu_v30_capabilities,
    run_gpu_or_cpu_grade_and_scopes,
};
pub use gpu_v70::{
    FAST_ACES_COLOR_MGMT_WGSL, FAST_BLEND_MASK_WGSL, FastBlendMaskParams, FastMaskShape,
    FramePoolBenchmarkReport, FramePoolBucketKey, FramePoolMetrics, GpuV70FastPipeline,
    GpuV70Status, PoolSurfaceFormat, ResidentFramePool, benchmark_v70_frame_pool,
    compose_ab_split_linear, detect_gpu_v70_status, execute_fast_blend_mask_cpu,
    execute_v70_aces_or_cpu, execute_v70_blend_mask_or_cpu, render_ab_compare_scene,
};
pub use gpu_v100::{
    FLUID_2D_ADVECTION_WGSL, GpuV100BenchmarkReport, GpuV100ComputePipeline,
    SPATIAL_360_EQUIRECT_WGSL, WgslValidationSummary, benchmark_v100_gpu_or_cpu,
    execute_v100_spatial_360_or_cpu, validate_all_builtin_wgsl_shaders,
};
pub use graph::{EFFECTS_WGSL_SOURCE, RenderGraph, render_frame_batch_parallel};
pub use hot_shader::{
    HotShaderEntry, ShaderHotReloader, ShaderReloadStatus, register_shader_dir, validate_wgsl,
};
pub use mask_tracker::{apply_bezier_masks, track_point_across_frames, track_point_in_scene};
pub use mesh3d_render::{
    CameraTrack3DSample, analyze_3d_camera_track, build_3d_camera_track_command,
    render_mesh_data_pbr, render_mesh_data_pbr_with_adv, render_mesh3d_item,
    render_mesh3d_item_with_adv,
};
pub use node_eval::{LayerRenderPacket, evaluate_node_composite_graph};
pub use pipeline::{
    COMPOSITE_COMPUTE_WGSL_SOURCE, GpuEffectUniform, GpuPostProcessPass, HeadlessGpuOffloader,
    WgpuComputeCompositor, WgpuPreviewPipeline,
};
pub use psd_sprite::PsdCharacterAsset;
pub use rig_render::{
    compute_frame_histogram_chi_sq, detect_scene_cuts_in_scene, render_rigged_puppet,
};
pub use sprite::{load_png_linear, render_placeholder_character, scale_linear_image};
pub use text::{StyledSpan, TextRenderer, parse_inline_text_spans, strip_inline_tags};
pub use v09_fx::{
    apply_3d_lighting_tint, build_drop_shadow_image, render_particle_emitter, render_vector_shape,
};
pub use v60_render::{apply_vfx_paint, render_data_chart};
pub use v80_render::{
    apply_cinema_optical_vfx, apply_warp_and_displacement, compute_sobel_edge_magnitude,
    draw_adr_countdown_streamer, estimate_homography_ransac, project_homography,
    propagate_roto_brush_in_scene, render_svg_morph, render_text_on_path,
    snap_roto_vertices_to_edges, solve_quad_to_quad_homography, track_planar_quad_in_scene,
};
pub use v90_render::{draw_live_tally_hud, render_live_stinger_transition, render_vtuber_avatar};
pub use v100_render::{
    render_cloth_sim_item, render_fluid_sim_item, render_spatial_360_and_stereo_pass,
};
pub use vfx_track::{
    CameraMotionSample, StabilizeConfig, analyze_camera_shake, apply_corner_pin,
    build_stabilize_commands, compute_shake_compensation, estimate_global_frame_motion, invert_3x3,
    solve_unit_square_to_quad_homography,
};
