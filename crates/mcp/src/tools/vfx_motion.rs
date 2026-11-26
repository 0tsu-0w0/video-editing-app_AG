//! v7.0 #46: 内蔵 MCP ドメインモジュール — VFX, Motion Graphics, 3D & GPU

#[allow(unused_imports)]
use super::*;

pub(super) const DOMAIN_ID: &str = "vfx_motion";
pub(super) const DOMAIN_LABEL: &str = "VFX, Motion Graphics, 3D & GPU";

pub(super) fn definitions() -> Vec<Value> {
    let mut out = Vec::new();
    if let Value::Array(items) = json!([
        {
            "name": "add_effect",
            "description": "オブジェクトの個別エフェクトチェーンへエフェクトを追加する",
            "inputSchema": {
                "type": "object",
                "required": ["id", "effect_type"],
                "properties": {
                    "id": { "type": "string" },
                    "effect_type": { "type": "string" },
                    "params": { "type": "object" }
                }
            }
        },
        {
            "name": "update_effect_params",
            "description": "オブジェクト上の既存エフェクトのパラメータを更新する",
            "inputSchema": {
                "type": "object",
                "required": ["id", "effect_id", "params"],
                "properties": {
                    "id": { "type": "string" },
                    "effect_id": { "type": "string" },
                    "params": { "type": "object" }
                }
            }
        },
        {
            "name": "set_keyframe",
            "description": "オブジェクトのプロパティへキーフレーム（ベジェ曲線イージング対応）を設定する",
            "inputSchema": {
                "type": "object",
                "required": ["id", "property", "value"],
                "properties": {
                    "id": { "type": "string" },
                    "property": { "type": "string" },
                    "time_sec": { "type": "number" },
                    "time_frame": { "type": "integer" },
                    "value": { "type": "number" },
                    "easing": { "type": "string", "enum": ["linear", "hold", "ease_out", "ease_in_out"] }
                }
            }
        },
        {
            "name": "precompose_selection",
            "description": "指定オブジェクト群をワンクリック結合（Smart Precompose）して新シーンへ移送する",
            "inputSchema": {
                "type": "object",
                "required": ["ids", "name"],
                "properties": {
                    "ids": { "type": "array", "items": { "type": "string" } },
                    "name": { "type": "string" },
                    "target_layer_index": { "type": "integer" }
                }
            }
        },
        {
            "name": "set_track_matte",
            "description": "指定レイヤーにトラックマット（alpha / alpha_inverted / luma / luma_inverted）を設定または解除する",
            "inputSchema": {
                "type": "object",
                "required": ["layer_index"],
                "properties": {
                    "layer_index": { "type": "integer" },
                    "mode": { "type": "string", "enum": ["alpha", "alpha_inverted", "luma", "luma_inverted"] },
                    "source_layer_index": { "type": "integer" }
                }
            }
        },
        {
            "name": "set_camera_3d",
            "description": "アクティブシーンの3Dカメラ位置・注視点・FOV・被写界深度を設定する",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "enabled": { "type": "boolean" },
                    "position": { "type": "array", "items": { "type": "number" } },
                    "target": { "type": "array", "items": { "type": "number" } },
                    "fov_deg": { "type": "number" },
                    "dof_enabled": { "type": "boolean" },
                    "focus_distance": { "type": "number" },
                    "aperture": { "type": "number" }
                }
            }
        },
        {
            "name": "add_character_sprite",
            "description": "指定音声レイヤーと自動連動するPNG差分立ち絵（自動口パク）オブジェクトを配置する",
            "inputSchema": {
                "type": "object",
                "required": ["layer_index", "name"],
                "properties": {
                    "layer_index": { "type": "integer" },
                    "name": { "type": "string" },
                    "linked_audio_layer": { "type": "integer" },
                    "mode": { "type": "string", "enum": ["volume_rms", "vowel_phoneme"] },
                    "start_sec": { "type": "number" },
                    "duration_sec": { "type": "number" }
                }
            }
        },
        {
            "name": "manage_effect",
            "description": "オブジェクトのエフェクトチェーンの並び替え・有効切替・削除を行う",
            "inputSchema": {
                "type": "object",
                "required": ["id", "action"],
                "properties": {
                    "id": { "type": "string" },
                    "action": { "type": "string", "enum": ["reorder", "remove", "toggle"] },
                    "effect_id": { "type": "string" },
                    "from_index": { "type": "integer" },
                    "to_index": { "type": "integer" },
                    "enabled": { "type": "boolean" }
                }
            }
        },
        {
            "name": "apply_motion_preset",
            "description": "指定オブジェクトへ登場・退場イージングや衝撃シェイクのワンクリックプリセットを適用する",
            "inputSchema": {
                "type": "object",
                "required": ["id", "preset"],
                "properties": {
                    "id": { "type": "string" },
                    "preset": { "type": "string", "enum": ["slide_in_left", "slide_in_bottom", "fade_in_out", "pop_bounce", "zoom_in", "impact_shake"] }
                }
            }
        },
        {
            "name": "set_bezier_mask",
            "description": "アイテムへ自由ベジェマスク（楕円/矩形/ひし形/カスタム頂点、加算・減算・反転・フェザー）を設定またはクリアする（Undo対応）",
            "inputSchema": {
                "type": "object",
                "required": ["id"],
                "properties": {
                    "id": { "type": "string" },
                    "action": { "type": "string", "enum": ["add_preset", "clear"], "default": "add_preset" },
                    "preset": { "type": "string", "enum": ["ellipse", "rectangle", "diamond"], "default": "ellipse" },
                    "mode": { "type": "string", "enum": ["add", "subtract"], "default": "add" },
                    "inverted": { "type": "boolean", "default": false },
                    "feather_px": { "type": "number", "default": 4.0 },
                    "opacity": { "type": "number", "default": 1.0 },
                    "vertices": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "x": { "type": "number" },
                                "y": { "type": "number" }
                            }
                        }
                    }
                }
            }
        }
    ]) {
        out.extend(items);
    }
    if let Value::Array(items) = json!([
        {
            "name": "manage_presets",
            "description": "字幕スタイルプリセット・エフェクトチェーンプリセット（presets/*.json＋組み込み）の一覧取得・保存・1クリック適用を行う（Undo対応）",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["list", "apply_text_style", "apply_effect_preset", "save_text_style", "save_effect_preset"]
                    },
                    "id": { "type": "string" },
                    "name": { "type": "string" }
                }
            }
        },
        {
            "name": "set_expression",
            "description": "Pure-Rust エクスプレッション（式アニメーション: wiggle/loopOut/audio_band/layer参照等）の設定・解除・一覧・即時評価・キーフレーム焼き込み(Bake)を行う（Undo対応）",
            "inputSchema": {
                "type": "object",
                "required": ["action", "id"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["set", "clear", "list", "evaluate", "bake"]
                    },
                    "id": { "type": "string" },
                    "property": { "type": "string" },
                    "expression": { "type": "string" },
                    "time_sec": { "type": "number" },
                    "step_frames": { "type": "integer", "default": 3 }
                }
            }
        },
        {
            "name": "manage_templates",
            "description": "モーショングラフィックス テンプレート（.mogrt 相当・レスポンシブ時間伸縮）およびプロジェクトテンプレートの一覧・配置・公開パラメータ更新・現シーン保存を行う（Undo対応）",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["list", "instantiate", "update_instance", "save_from_scene", "list_project_templates", "save_project_template"]
                    },
                    "template_id": { "type": "string" },
                    "name": { "type": "string" },
                    "category": { "type": "string" },
                    "layer_index": { "type": "integer", "default": 1 },
                    "start_sec": { "type": "number", "default": 0.0 },
                    "duration_sec": { "type": "number" },
                    "intro_sec": { "type": "number", "default": 0.45 },
                    "outro_sec": { "type": "number", "default": 0.45 },
                    "item_id": { "type": "string" },
                    "values": { "type": "object" }
                }
            }
        },
        {
            "name": "set_scene_motion_blur",
            "description": "シャッター角度ベースモーションブラー、3D ディレクショナル／ポイントライト＆ドロップシャドウキャッチャー、およびパーティクル／ベクターシェイプ設定を操作する（Undo対応）",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["get", "set_motion_blur", "set_light_3d", "set_particle", "set_vector_shape"]
                    },
                    "enabled": { "type": "boolean" },
                    "shutter_angle_deg": { "type": "number" },
                    "shutter_phase_deg": { "type": "number" },
                    "preview_samples": { "type": "integer" },
                    "export_samples": { "type": "integer" },
                    "light_type": { "type": "string", "enum": ["directional", "point"] },
                    "position": { "type": "array", "items": { "type": "number" } },
                    "intensity": { "type": "number" },
                    "ambient": { "type": "number" },
                    "cast_shadow": { "type": "boolean" },
                    "shadow_softness": { "type": "number" },
                    "shadow_opacity": { "type": "number" },
                    "item_id": { "type": "string" },
                    "preset": { "type": "string" },
                    "count": { "type": "integer" },
                    "seed": { "type": "integer" },
                    "speed": { "type": "number" },
                    "size_px": { "type": "number" },
                    "trim_start": { "type": "number" },
                    "trim_end": { "type": "number" },
                    "trim_offset": { "type": "number" },
                    "stroke_width": { "type": "number" },
                    "dash_length": { "type": "number" },
                    "dash_gap": { "type": "number" }
                }
            }
        },
        {
            "name": "manage_node_graph",
            "description": "v2.0 #21: 第3ワークスペース（Fusion/Nukeスタイル・ノードベース合成グラフ）の生成・双方向レイヤー同期・ノード追加/接続/削除・有効化を行う",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["get_graph", "from_layers", "sync_to_layers", "set_enabled", "add_node", "connect", "remove_node", "validate"]
                    },
                    "enabled": { "type": "boolean" },
                    "node_type": {
                        "type": "string",
                        "enum": ["layer_input", "solid_color", "effect", "channel_split", "channel_merge", "transform", "merge", "mask_combine", "color_correct", "depth_fog", "output"]
                    },
                    "name": { "type": "string" },
                    "layer_index": { "type": "integer" },
                    "effect_type": { "type": "string" },
                    "channel": { "type": "string", "enum": ["rgba", "red", "green", "blue", "alpha", "luma", "depth"] },
                    "blend_mode": { "type": "string", "enum": ["normal", "add", "multiply", "screen"] },
                    "matte_mode": { "type": "string", "enum": ["alpha", "alpha_inverted", "luma", "luma_inverted"] },
                    "params": { "type": "object" },
                    "from_node": { "type": "string" },
                    "from_port": { "type": "string" },
                    "to_node": { "type": "string" },
                    "to_port": { "type": "string" },
                    "node_id": { "type": "string" }
                }
            }
        },
        {
            "name": "vfx_stabilize_and_corner_pin",
            "description": "v2.1 #26: 自動2D手ぶれ補正（ゼロ位相平滑化＋自動クロップ拡大キーフレーム生成）＆ 平面4点コーナーピン射影合成（3x3ホモグラフィ）を実行する",
            "inputSchema": {
                "type": "object",
                "required": ["action", "id"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["analyze_and_stabilize", "set_corner_pin", "reset_corner_pin"]
                    },
                    "id": { "type": "string" },
                    "smoothness": { "type": "number" },
                    "max_shift_ratio": { "type": "number" },
                    "sample_interval_sec": { "type": "number" },
                    "stabilize_rotation": { "type": "boolean" },
                    "auto_crop_zoom": { "type": "boolean" },
                    "apply": { "type": "boolean" },
                    "enabled": { "type": "boolean" },
                    "preset": {
                        "type": "string",
                        "enum": ["reset", "monitor_tilt", "billboard_left", "perspective_floor"]
                    },
                    "tl": { "type": "array" },
                    "tr": { "type": "array" },
                    "br": { "type": "array" },
                    "bl": { "type": "array" },
                    "feather_px": { "type": "number" }
                }
            }
        },
        {
            "name": "gpu_v30_pipeline",
            "description": "v3.0 #29: フル GPU コンピュートパイプライン（Vulkan/Metal/DX12 WGSL コンピュートシェーダー検証・4K ベンチマーク・GPU/CPU フォールバック付きリアルタイム色調＆ビデオスコープ実行）を操作する",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["status", "benchmark_4k", "run_grade_and_scopes"]
                    },
                    "width": { "type": "integer" },
                    "height": { "type": "integer" },
                    "time_sec": { "type": "number" }
                }
            }
        },
        {
            "name": "scene_3d_and_camera_track",
            "description": "v3.0 #30: 3D メッシュ / glTF 2.0 (`.gltf`/`.glb`) / 3D 立体押し出しテキストの配置・PBR 質感更新・サンプル GLB 書き出し＆検証・視差ベース 3D カメラトラッキングを実行する",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["add_mesh_3d", "update_mesh_3d", "emit_sample_glb", "track_3d_camera"]
                    },
                    "id": { "type": "string" },
                    "layer_index": { "type": "integer" },
                    "name": { "type": "string" },
                    "preset": {
                        "type": "string",
                        "enum": ["cube", "sphere", "torus", "extruded_text", "gltf_file"]
                    },
                    "gltf_path": { "type": "string" },
                    "text_content": { "type": "string" },
                    "extrude_depth": { "type": "number" },
                    "base_color": { "type": "string" },
                    "metallic": { "type": "number" },
                    "roughness": { "type": "number" },
                    "env_reflection": { "type": "number" },
                    "wireframe": { "type": "boolean" },
                    "auto_spin_deg_per_sec": { "type": "number" },
                    "start_sec": { "type": "number" },
                    "duration_sec": { "type": "number" },
                    "sample_interval_sec": { "type": "number" },
                    "apply": { "type": "boolean" }
                }
            }
        },
        {
            "name": "physics_simulate_and_bake",
            "description": "v4.0 #34: 決定論的 60Hz リジッドボディ物理演算シミュレーション（重力バウンス・ドミノ連鎖衝突・無重力浮遊・軌道ボテックス）を実行し、軌跡プレビュー取得または 1-Undo キーフレーム自動ベイクを行う",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["preview_trajectory", "bake_keyframes"]
                    },
                    "item_ids": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "preset": {
                        "type": "string",
                        "enum": ["bouncing_title", "domino_cascade", "zero_gravity_float", "orbital_vortex"]
                    },
                    "gravity_y": { "type": "number" },
                    "restitution": { "type": "number" },
                    "friction": { "type": "number" },
                    "air_drag": { "type": "number" },
                    "initial_vx": { "type": "number" },
                    "initial_vy": { "type": "number" },
                    "initial_spin_deg": { "type": "number" },
                    "central_attract": { "type": "number" },
                    "duration_sec": { "type": "number" },
                    "bake_step_frames": { "type": "integer" },
                    "floor_ratio": { "type": "number" }
                }
            }
        },
        {
            "name": "generative_bg_and_foley",
            "description": "v4.0 #35: オフライン・プロシージャル背景ジェネレーター（オーロラ・シネマ玉ボケ・サイバー3Dグリッド・リキッドマーブル・ワープ星雲・アブストラクト波）の配置・編集と、プロシージャル Foley/SFX 効果音（ウーッシュ・重低音インパクト・UIポップ・グリッチ・雨音・ライザー）48kHz ステレオ WAV 合成＆タイムライン配置を行う",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["add_generative_bg", "update_generative_bg", "synthesize_foley_sfx"]
                    },
                    "id": { "type": "string" },
                    "name": { "type": "string" },
                    "preset": { "type": "string" },
                    "layer_index": { "type": "integer" },
                    "audio_layer_index": { "type": "integer" },
                    "start_sec": { "type": "number" },
                    "duration_sec": { "type": "number" },
                    "seed": { "type": "integer" },
                    "speed": { "type": "number" },
                    "scale": { "type": "number" },
                    "complexity": { "type": "integer" },
                    "width": { "type": "number" },
                    "height": { "type": "number" },
                    "apply": { "type": "boolean" }
                }
            }
        }
    ]) {
        out.extend(items);
    }
    if let Value::Array(items) = json!([
        {
            "name": "rig2d_puppet_control",
            "description": "v5.0 #37: 2D スケルトンボーン＆2-Bone IK パペットリグ（階層 FK・解析的 2-Bone 逆運動学・髪/リボン二次バネ揺れ・4種モーションプリセット）の配置・編集・IK 姿勢プレビューを行う",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["add_rigged_puppet", "update_rig", "solve_ik_preview"]
                    },
                    "id": { "type": "string" },
                    "name": { "type": "string" },
                    "preset": {
                        "type": "string",
                        "enum": ["idle_breathe", "wave_hello", "nod_talk", "dance_bounce"]
                    },
                    "layer_index": { "type": "integer" },
                    "start_sec": { "type": "number" },
                    "duration_sec": { "type": "number" },
                    "motion_speed": { "type": "number" },
                    "motion_amp": { "type": "number" },
                    "show_bones_overlay": { "type": "boolean" },
                    "right_ik_enabled": { "type": "boolean" },
                    "right_ik_x": { "type": "number" },
                    "right_ik_y": { "type": "number" },
                    "left_ik_enabled": { "type": "boolean" },
                    "left_ik_x": { "type": "number" },
                    "left_ik_y": { "type": "number" },
                    "time_sec": { "type": "number" }
                }
            }
        },
        {
            "name": "data_chart_motion_graphics",
            "description": "v6.0 #41: データ駆動モーショングラフィックス＆インフォグラフィック・チャート（棒グラフ・折れ線・ドーナツ・KPIカウンター / CSV取り込み）を生成・更新する",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["create_chart", "update_chart", "import_csv"]
                    },
                    "item_id": { "type": "string" },
                    "kind": {
                        "type": "string",
                        "enum": ["bar_chart", "line_chart", "donut_chart", "kpi_counter"]
                    },
                    "layer_index": { "type": "integer" },
                    "name": { "type": "string" },
                    "title": { "type": "string" },
                    "unit_suffix": { "type": "string" },
                    "csv_text": { "type": "string" },
                    "points": { "type": "array" },
                    "start_sec": { "type": "number" },
                    "duration_sec": { "type": "number" },
                    "animate_in_sec": { "type": "number" },
                    "width": { "type": "number" },
                    "height": { "type": "number" },
                    "x": { "type": "number" },
                    "y": { "type": "number" },
                    "accent_color": { "type": "string" },
                    "bg_color": { "type": "string" },
                    "show_grid": { "type": "boolean" },
                    "show_values": { "type": "boolean" }
                }
            }
        },
        {
            "name": "vfx_paint_and_inpaint",
            "description": "v6.0 #42: 非破壊 VFX ペイント（Write-On グローブラシ・クローンスタンプ・ワイヤー/不要物消しインペインティング・プライバシー秘匿ぼかし）をクリップへ設定・追加する",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["add_stroke", "set_paint", "clear_paint", "get_paint"]
                    },
                    "item_id": { "type": "string" },
                    "tool": {
                        "type": "string",
                        "enum": ["brush_glow", "clone_stamp", "wire_erase_inpaint", "blur_redact"]
                    },
                    "points": { "type": "array" },
                    "radius_px": { "type": "number" },
                    "hardness": { "type": "number" },
                    "opacity": { "type": "number" },
                    "color": { "type": "string" },
                    "clone_offset": { "type": "array" },
                    "write_on_progress": { "type": "number" },
                    "animate_write_on": { "type": "boolean" },
                    "enabled": { "type": "boolean" }
                },
                "required": ["item_id"]
            }
        }
    ]) {
        out.extend(items);
    }
    out
}

pub(super) fn try_call(
    guard: &mut crate::session::EditorSession,
    fps: FrameRate,
    name: &str,
    args: &Value,
) -> Option<Result<Value, String>> {
    if !matches!(
        name,
        "add_effect"
            | "update_effect_params"
            | "set_keyframe"
            | "precompose_selection"
            | "set_track_matte"
            | "set_camera_3d"
            | "add_character_sprite"
            | "manage_effect"
            | "apply_motion_preset"
            | "set_bezier_mask"
            | "manage_presets"
            | "set_expression"
            | "manage_templates"
            | "set_scene_motion_blur"
            | "manage_node_graph"
            | "vfx_stabilize_and_corner_pin"
            | "gpu_v30_pipeline"
            | "scene_3d_and_camera_track"
            | "physics_simulate_and_bake"
            | "generative_bg_and_foley"
            | "rig2d_puppet_control"
            | "data_chart_motion_graphics"
            | "vfx_paint_and_inpaint"
    ) {
        return None;
    }
    Some(call_inner(guard, fps, name, args))
}

fn call_inner(
    guard: &mut crate::session::EditorSession,
    fps: FrameRate,
    name: &str,
    args: &Value,
) -> Result<Value, String> {
    match name {
        "add_effect" => {
            let id = parse_uuid(args, "id")?;
            let effect_type = args
                .get("effect_type")
                .and_then(Value::as_str)
                .ok_or("missing effect_type")?
                .to_string();
            let effect_id = Uuid::new_v4();
            let effect = EffectNode {
                id: effect_id,
                effect_type,
                enabled: true,
                params: parse_params_map(args.get("params")),
            };
            let sess = &mut *guard;
            sess.bus
                .execute(
                    &mut sess.project,
                    EditCommand::AddEffect {
                        item_id: id,
                        effect,
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Ok(
                json!({ "content": [{ "type": "text", "text": json!({"effect_id": effect_id}).to_string() }] }),
            )
        }
        "update_effect_params" => {
            let id = parse_uuid(args, "id")?;
            let effect_id = parse_uuid(args, "effect_id")?;
            let params = parse_params_map(args.get("params"));
            let sess = &mut *guard;
            sess.bus
                .execute(
                    &mut sess.project,
                    EditCommand::UpdateEffectParams {
                        item_id: id,
                        effect_id,
                        params,
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Ok(json!({ "content": [{ "type": "text", "text": "updated" }] }))
        }
        "set_keyframe" => {
            let id = parse_uuid(args, "id")?;
            let property = args
                .get("property")
                .and_then(Value::as_str)
                .ok_or("missing property")?
                .to_string();
            let value = args
                .get("value")
                .and_then(Value::as_f64)
                .ok_or("missing value")? as f32;
            let time = parse_time_arg(args, "time_frame", "time_sec", fps, RationalTime::ZERO);
            let interpolation = match args.get("easing").and_then(Value::as_str) {
                Some("hold") => Interpolation::Hold,
                Some("ease_out") => Interpolation::Bezier(CubicBezier::EASE_OUT),
                Some("ease_in_out") => Interpolation::Bezier(CubicBezier::EASE_IN_OUT),
                _ => Interpolation::Linear,
            };
            let sess = &mut *guard;
            sess.bus
                .execute(
                    &mut sess.project,
                    EditCommand::SetKeyframe {
                        item_id: id,
                        property,
                        keyframe: Keyframe {
                            time,
                            value,
                            interpolation,
                        },
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Ok(json!({ "content": [{ "type": "text", "text": "keyframe_set" }] }))
        }
        "precompose_selection" => {
            let ids_arr = args
                .get("ids")
                .and_then(Value::as_array)
                .ok_or("missing ids array")?;
            let mut item_ids = Vec::with_capacity(ids_arr.len());
            for v in ids_arr {
                let s = v.as_str().ok_or("id must be string")?;
                item_ids.push(Uuid::parse_str(s).map_err(|e| e.to_string())?);
            }
            let precomp_name = args
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("Precomp")
                .to_string();
            let target_layer_index = args
                .get("target_layer_index")
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize;
            let new_scene_id = Uuid::new_v4();
            let precomp_item_id = Uuid::new_v4();
            let sess = &mut *guard;
            sess.bus
                .execute(
                    &mut sess.project,
                    EditCommand::PrecomposeSelection {
                        item_ids,
                        new_scene_id,
                        precomp_name,
                        precomp_item_id,
                        target_layer_index,
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": json!({"scene_id": new_scene_id, "precomp_item_id": precomp_item_id}).to_string()
                }]
            }))
        }
        "set_track_matte" => {
            let layer_index = args
                .get("layer_index")
                .and_then(Value::as_u64)
                .ok_or("missing layer_index")? as usize;
            let matte = if let (Some(mode_s), Some(src_idx)) = (
                args.get("mode").and_then(Value::as_str),
                args.get("source_layer_index").and_then(Value::as_u64),
            ) {
                let mode = match mode_s {
                    "alpha_inverted" => TrackMatteMode::AlphaInverted,
                    "luma" => TrackMatteMode::Luma,
                    "luma_inverted" => TrackMatteMode::LumaInverted,
                    _ => TrackMatteMode::Alpha,
                };
                Some(TrackMatte {
                    mode,
                    source_layer_index: src_idx as usize,
                })
            } else {
                None
            };
            let sess = &mut *guard;
            sess.bus
                .execute(
                    &mut sess.project,
                    EditCommand::SetTrackMatte { layer_index, matte },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Ok(json!({ "content": [{ "type": "text", "text": "track_matte_updated" }] }))
        }
        "set_camera_3d" => {
            let mut cam = guard
                .project
                .active_scene()
                .map(|s| s.camera)
                .unwrap_or_default();
            if let Some(en) = args.get("enabled").and_then(Value::as_bool) {
                cam.enabled = en;
            }
            if let Some(arr) = args.get("position").and_then(Value::as_array)
                && arr.len() == 3
            {
                cam.position = [
                    arr[0].as_f64().unwrap_or(0.0) as f32,
                    arr[1].as_f64().unwrap_or(0.0) as f32,
                    arr[2].as_f64().unwrap_or(-1000.0) as f32,
                ];
            }
            if let Some(arr) = args.get("target").and_then(Value::as_array)
                && arr.len() == 3
            {
                cam.target = [
                    arr[0].as_f64().unwrap_or(0.0) as f32,
                    arr[1].as_f64().unwrap_or(0.0) as f32,
                    arr[2].as_f64().unwrap_or(0.0) as f32,
                ];
            }
            if let Some(fov) = args.get("fov_deg").and_then(Value::as_f64)
                && fov.is_finite()
            {
                cam.fov_deg = fov.clamp(5.0, 150.0) as f32;
            }
            if let Some(dof) = args.get("dof_enabled").and_then(Value::as_bool) {
                cam.dof_enabled = dof;
            }
            if let Some(fd) = args.get("focus_distance").and_then(Value::as_f64)
                && fd.is_finite()
            {
                cam.focus_distance = fd.clamp(1.0, 100_000.0) as f32;
            }
            if let Some(ap) = args.get("aperture").and_then(Value::as_f64)
                && ap.is_finite()
            {
                cam.aperture = ap.clamp(0.0, 1.0) as f32;
            }
            let sess = &mut *guard;
            sess.bus
                .execute(
                    &mut sess.project,
                    EditCommand::SetCamera3D { camera: cam },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Ok(json!({ "content": [{ "type": "text", "text": "camera_3d_updated" }] }))
        }
        "add_character_sprite" => {
            let layer_index = args
                .get("layer_index")
                .and_then(Value::as_u64)
                .ok_or("missing layer_index")? as usize;
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("立ち絵")
                .to_string();
            let linked_audio_layer = args
                .get("linked_audio_layer")
                .and_then(Value::as_u64)
                .map(|v| v as usize);
            let mode = match args.get("mode").and_then(Value::as_str) {
                Some("vowel_phoneme") => LipSyncMode::VowelPhoneme,
                _ => LipSyncMode::default(),
            };
            let start = RationalTime::from_seconds(finite_clamped(
                args.get("start_sec"),
                0.0,
                0.0,
                86_400.0,
            ));
            let duration = RationalTime::from_seconds(finite_clamped(
                args.get("duration_sec"),
                5.0,
                0.01,
                86_400.0,
            ));
            let item = TimelineItem::new(
                name,
                start,
                duration,
                ItemKind::CharacterSprite {
                    config: CharacterSpriteConfig {
                        linked_audio_layer,
                        mode,
                        png_paths: BTreeMap::new(),
                        psd_config: None,
                        phoneme_override: Vec::new(),
                    },
                    width: 260.0,
                    height: 320.0,
                },
            );
            let id = item.id;
            let sess = &mut *guard;
            sess.bus
                .execute(
                    &mut sess.project,
                    EditCommand::AddObject { layer_index, item },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Ok(
                json!({ "content": [{ "type": "text", "text": json!({"created_id": id}).to_string() }] }),
            )
        }
        "manage_effect" => {
            let item_id = parse_uuid(args, "id")?;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or("missing 'action'")?;
            let cmd = match action {
                "reorder" => EditCommand::ReorderEffect {
                    item_id,
                    from_index: args
                        .get("from_index")
                        .and_then(Value::as_u64)
                        .ok_or("missing 'from_index'")? as usize,
                    to_index: args
                        .get("to_index")
                        .and_then(Value::as_u64)
                        .ok_or("missing 'to_index'")? as usize,
                },
                "remove" => EditCommand::RemoveEffect {
                    item_id,
                    effect_id: parse_uuid(args, "effect_id")?,
                },
                "toggle" => EditCommand::ToggleEffect {
                    item_id,
                    effect_id: parse_uuid(args, "effect_id")?,
                    enabled: args.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                },
                other => return Err(format!("unknown action: {other}")),
            };
            let sess = &mut *guard;
            sess.bus
                .execute(&mut sess.project, cmd, CommandSource::Mcp, None)
                .map_err(|e| e.to_string())?;
            Ok(
                json!({ "content": [{ "type": "text", "text": json!({"ok": true, "action": action}).to_string() }] }),
            )
        }
        "apply_motion_preset" => {
            let item_id = parse_uuid(args, "id")?;
            let preset_name = args
                .get("preset")
                .and_then(Value::as_str)
                .ok_or("missing 'preset'")?;
            let preset = match preset_name {
                "slide_in_left" => MotionPreset::SlideInLeft,
                "slide_in_bottom" => MotionPreset::SlideInBottom,
                "fade_in_out" => MotionPreset::FadeInOut,
                "pop_bounce" => MotionPreset::PopBounce,
                "zoom_in" => MotionPreset::ZoomIn,
                "impact_shake" => MotionPreset::ImpactShake,
                other => return Err(format!("unknown preset: {other}")),
            };
            let item = guard
                .project
                .active_scene()
                .and_then(|s| {
                    s.layers
                        .iter()
                        .flat_map(|l| l.items.iter())
                        .find(|i| i.id == item_id)
                        .cloned()
                })
                .ok_or("item not found")?;
            let cmd = build_motion_preset_commands(&item, preset);
            let sess = &mut *guard;
            sess.bus
                .execute(&mut sess.project, cmd, CommandSource::Mcp, None)
                .map_err(|e| e.to_string())?;
            Ok(
                json!({ "content": [{ "type": "text", "text": json!({"applied": preset_name}).to_string() }] }),
            )
        }
        "set_bezier_mask" => {
            let item_id = parse_uuid(args, "id")?;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or("add_preset");
            let mut masks = guard
                .project
                .active_scene()
                .and_then(|s| {
                    s.layers
                        .iter()
                        .flat_map(|l| l.items.iter())
                        .find(|i| i.id == item_id)
                        .map(|i| i.masks.clone())
                })
                .ok_or_else(|| format!("object not found: {item_id}"))?;

            if action == "clear" {
                masks.clear();
            } else {
                let mut mask = match args.get("preset").and_then(Value::as_str) {
                    Some("rectangle") => BezierMask::rectangle(),
                    Some("diamond") => BezierMask::diamond(),
                    _ => BezierMask::ellipse(),
                };
                if let Some(m) = args.get("mode").and_then(Value::as_str) {
                    mask.mode = if m == "subtract" {
                        MaskMode::Subtract
                    } else {
                        MaskMode::Add
                    };
                }
                if let Some(inv) = args.get("inverted").and_then(Value::as_bool) {
                    mask.inverted = inv;
                }
                mask.feather_px = finite_clamped(
                    args.get("feather_px"),
                    f64::from(mask.feather_px),
                    0.0,
                    96.0,
                ) as f32;
                mask.opacity =
                    finite_clamped(args.get("opacity"), f64::from(mask.opacity), 0.0, 1.0) as f32;
                if let Some(verts) = args.get("vertices").and_then(Value::as_array)
                    && verts.len() >= 3
                {
                    let parsed: Vec<MaskVertex> = verts
                        .iter()
                        .filter_map(|v| {
                            let x = v.get("x")?.as_f64()? as f32;
                            let y = v.get("y")?.as_f64()? as f32;
                            if x.is_finite() && y.is_finite() {
                                Some(MaskVertex::corner(x.clamp(-0.5, 1.5), y.clamp(-0.5, 1.5)))
                            } else {
                                None
                            }
                        })
                        .collect();
                    if parsed.len() >= 3 {
                        mask.vertices = parsed;
                    }
                }
                masks.push(mask);
            }

            let count = masks.len();
            let s = &mut *guard;
            s.bus
                .execute(
                    &mut s.project,
                    EditCommand::SetMasks { item_id, masks },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": json!({ "item_id": item_id, "mask_count": count }).to_string()
                }]
            }))
        }
        "manage_presets" => {
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or("missing 'action'")?;
            match action {
                "list" => {
                    let mut text_presets = TextStylePreset::default_presets();
                    let mut effect_presets = EffectPreset::default_presets();
                    if let Some(folder) = guard.folder.as_ref() {
                        text_presets.extend(folder.load_text_style_presets());
                        effect_presets.extend(folder.load_effect_presets());
                    }
                    let text_names: Vec<&str> =
                        text_presets.iter().map(|p| p.name.as_str()).collect();
                    let fx_names: Vec<&str> =
                        effect_presets.iter().map(|p| p.name.as_str()).collect();
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "text_style_presets": text_names,
                                "effect_presets": fx_names,
                            }).to_string()
                        }]
                    }))
                }
                "apply_text_style" => {
                    let item_id = parse_uuid(args, "id")?;
                    let preset_name = args
                        .get("name")
                        .and_then(Value::as_str)
                        .ok_or("missing 'name'")?;
                    let preset = guard
                        .folder
                        .as_ref()
                        .and_then(|f| {
                            f.load_text_style_presets()
                                .into_iter()
                                .find(|p| p.name == preset_name)
                        })
                        .or_else(|| {
                            TextStylePreset::default_presets()
                                .into_iter()
                                .find(|p| p.name == preset_name)
                        })
                        .ok_or_else(|| format!("text style preset not found: {preset_name}"))?;
                    let batch = EditCommand::Batch {
                        label: format!("apply_text_style:{preset_name}"),
                        commands: vec![
                            EditCommand::SetTextFont {
                                item_id,
                                font_family: preset.font_family,
                                font_size: preset.font_size,
                            },
                            EditCommand::SetItemColor {
                                item_id,
                                color: preset.color,
                            },
                            EditCommand::SetTextStyle {
                                item_id,
                                style: preset.text_style,
                            },
                        ],
                    };
                    let sess = &mut *guard;
                    sess.bus
                        .execute(&mut sess.project, batch, CommandSource::Mcp, None)
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({"applied_text_style": preset_name, "id": item_id}).to_string()
                        }]
                    }))
                }
                "apply_effect_preset" => {
                    let item_id = parse_uuid(args, "id")?;
                    let preset_name = args
                        .get("name")
                        .and_then(Value::as_str)
                        .ok_or("missing 'name'")?;
                    let preset = guard
                        .folder
                        .as_ref()
                        .and_then(|f| {
                            f.load_effect_presets()
                                .into_iter()
                                .find(|p| p.name == preset_name)
                        })
                        .or_else(|| {
                            EffectPreset::default_presets()
                                .into_iter()
                                .find(|p| p.name == preset_name)
                        })
                        .ok_or_else(|| format!("effect preset not found: {preset_name}"))?;
                    let effects = preset.instantiate();
                    let count = effects.len();
                    let sess = &mut *guard;
                    sess.bus
                        .execute(
                            &mut sess.project,
                            EditCommand::PasteEffects { item_id, effects },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({"applied_effect_preset": preset_name, "added_effects": count}).to_string()
                        }]
                    }))
                }
                "save_text_style" => {
                    let item_id = parse_uuid(args, "id")?;
                    let preset_name = args
                        .get("name")
                        .and_then(Value::as_str)
                        .filter(|s| !s.trim().is_empty())
                        .ok_or("missing 'name'")?;
                    let folder = guard
                        .folder
                        .as_ref()
                        .ok_or("プロジェクトフォルダが未保存のためプリセットを保存できません")?;
                    let item = guard
                        .project
                        .active_scene()
                        .and_then(|s| {
                            s.layers
                                .iter()
                                .flat_map(|l| l.items.iter())
                                .find(|i| i.id == item_id)
                        })
                        .ok_or_else(|| format!("object not found: {item_id}"))?;
                    let ItemKind::Text {
                        font_family,
                        font_size,
                        color,
                        ..
                    } = &item.kind
                    else {
                        return Err("指定オブジェクトはテキストではありません".into());
                    };
                    let preset = TextStylePreset {
                        name: preset_name.trim().to_string(),
                        font_family: font_family.clone(),
                        font_size: *font_size,
                        color: *color,
                        text_style: item.text_style,
                    };
                    let path = folder
                        .save_text_style_preset(&preset)
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({"saved_text_style": preset.name, "path": path.display().to_string()}).to_string()
                        }]
                    }))
                }
                "save_effect_preset" => {
                    let item_id = parse_uuid(args, "id")?;
                    let preset_name = args
                        .get("name")
                        .and_then(Value::as_str)
                        .filter(|s| !s.trim().is_empty())
                        .ok_or("missing 'name'")?;
                    let folder = guard
                        .folder
                        .as_ref()
                        .ok_or("プロジェクトフォルダが未保存のためプリセットを保存できません")?;
                    let item = guard
                        .project
                        .active_scene()
                        .and_then(|s| {
                            s.layers
                                .iter()
                                .flat_map(|l| l.items.iter())
                                .find(|i| i.id == item_id)
                        })
                        .ok_or_else(|| format!("object not found: {item_id}"))?;
                    let preset = EffectPreset {
                        name: preset_name.trim().to_string(),
                        effects: item.effect_chain.clone(),
                    };
                    let path = folder
                        .save_effect_preset(&preset)
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({"saved_effect_preset": preset.name, "path": path.display().to_string()}).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown preset action: {other}")),
            }
        }
        "set_expression" => {
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or("missing action")?;
            let id = parse_uuid(args, "id")?;
            match action {
                "list" => {
                    let scene = guard.project.active_scene().ok_or("no active scene")?;
                    let item = scene
                        .layers
                        .iter()
                        .flat_map(|l| l.items.iter())
                        .find(|i| i.id == id)
                        .ok_or_else(|| format!("object not found: {id}"))?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "item_id": id,
                                "expressions": item.expressions
                            }).to_string()
                        }]
                    }))
                }
                "set" | "clear" => {
                    let property = args
                        .get("property")
                        .and_then(Value::as_str)
                        .ok_or("missing property")?
                        .to_string();
                    let expr = if action == "clear" {
                        None
                    } else {
                        let s = args
                            .get("expression")
                            .and_then(Value::as_str)
                            .ok_or("missing expression")?;
                        compile_expression(s).map_err(|e| e.to_string())?;
                        Some(s.to_string())
                    };
                    let sess = &mut *guard;
                    sess.bus
                        .execute(
                            &mut sess.project,
                            EditCommand::SetExpression {
                                item_id: id,
                                property: property.clone(),
                                expression: expr.clone(),
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "item_id": id,
                                "property": property,
                                "expression": expr
                            }).to_string()
                        }]
                    }))
                }
                "evaluate" => {
                    let property = args
                        .get("property")
                        .and_then(Value::as_str)
                        .ok_or("missing property")?;
                    let t = args
                        .get("time_sec")
                        .and_then(Value::as_f64)
                        .map(RationalTime::from_seconds)
                        .unwrap_or(guard.playhead);
                    let scene = guard.project.active_scene().ok_or("no active scene")?;
                    let (l_idx, item) = scene
                        .layers
                        .iter()
                        .enumerate()
                        .find_map(|(li, l)| l.items.iter().find(|i| i.id == id).map(|it| (li, it)))
                        .ok_or_else(|| format!("object not found: {id}"))?;
                    let val = evaluate_item_property(scene, l_idx, item, property, t, [0.0; 4]);
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "item_id": id,
                                "property": property,
                                "time_sec": t.as_seconds(),
                                "value": val
                            }).to_string()
                        }]
                    }))
                }
                "bake" => {
                    let property = args
                        .get("property")
                        .and_then(Value::as_str)
                        .ok_or("missing property")?;
                    let step_frames = args
                        .get("step_frames")
                        .and_then(Value::as_u64)
                        .unwrap_or(3)
                        .clamp(1, 60) as u32;
                    let cmd = {
                        let scene = guard.project.active_scene().ok_or("no active scene")?;
                        let (l_idx, item) = scene
                            .layers
                            .iter()
                            .enumerate()
                            .find_map(|(li, l)| {
                                l.items.iter().find(|i| i.id == id).map(|it| (li, it))
                            })
                            .ok_or_else(|| format!("object not found: {id}"))?;
                        build_bake_expression_command(
                            scene,
                            l_idx,
                            item,
                            property,
                            step_frames,
                            |_| [0.0; 4],
                        )?
                    };
                    let sess = &mut *guard;
                    sess.bus
                        .execute(&mut sess.project, cmd, CommandSource::Mcp, None)
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "baked": true,
                                "item_id": id,
                                "property": property,
                                "step_frames": step_frames
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown set_expression action: {other}")),
            }
        }
        "manage_templates" => {
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or("missing action")?;
            let all_motion_templates = || {
                let mut list = builtin_motion_templates();
                if let Some(folder) = guard.folder.as_ref() {
                    list.extend(folder.load_motion_templates());
                }
                list
            };
            match action {
                "list" => {
                    let list = all_motion_templates();
                    let summary: Vec<Value> = list
                        .iter()
                        .map(|t| {
                            json!({
                                "id": t.id,
                                "name": t.name,
                                "category": t.category,
                                "description": t.description,
                                "default_duration_sec": t.responsive_time.default_duration_sec,
                                "exposed_params": t.exposed_params
                            })
                        })
                        .collect();
                    Ok(json!({
                        "content": [{ "type": "text", "text": json!({ "templates": summary }).to_string() }]
                    }))
                }
                "instantiate" => {
                    let tpl_id = args
                        .get("template_id")
                        .and_then(Value::as_str)
                        .ok_or("missing template_id")?;
                    let list = all_motion_templates();
                    let tpl = list
                        .iter()
                        .find(|t| t.id == tpl_id || t.name == tpl_id)
                        .ok_or_else(|| format!("template not found: {tpl_id}"))?
                        .clone();
                    let layer_index =
                        args.get("layer_index").and_then(Value::as_u64).unwrap_or(1) as usize;
                    let start = RationalTime::from_seconds(finite_clamped(
                        args.get("start_sec"),
                        guard.playhead.as_seconds(),
                        0.0,
                        86_400.0,
                    ));
                    let dur_opt = args.get("duration_sec").and_then(Value::as_f64);
                    let overrides = parse_template_param_values(args.get("values"));
                    let cmd = build_instantiate_template_command(
                        &tpl,
                        layer_index,
                        start,
                        dur_opt,
                        &overrides,
                    );
                    let sess = &mut *guard;
                    sess.bus
                        .execute(&mut sess.project, cmd, CommandSource::Mcp, None)
                        .map_err(|e| e.to_string())?;
                    let created_id = sess
                        .project
                        .active_scene()
                        .and_then(|s| s.layers.get(layer_index))
                        .and_then(|l| l.items.last())
                        .map(|i| i.id);
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "instantiated": tpl.id,
                                "created_item_id": created_id
                            }).to_string()
                        }]
                    }))
                }
                "update_instance" => {
                    let item_id = parse_uuid(args, "item_id")?;
                    let values = parse_template_param_values(args.get("values"));
                    let duration_sec = args.get("duration_sec").and_then(Value::as_f64);
                    let sess = &mut *guard;
                    sess.bus
                        .execute(
                            &mut sess.project,
                            EditCommand::UpdateTemplateInstance {
                                item_id,
                                values,
                                duration_sec,
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": json!({ "updated": item_id }).to_string() }]
                    }))
                }
                "save_from_scene" => {
                    let folder = guard
                        .folder
                        .as_ref()
                        .ok_or("project folder not initialized")?
                        .clone();
                    let scene = guard
                        .project
                        .active_scene()
                        .ok_or("no active scene")?
                        .clone();
                    let name = args
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("カスタムテンプレート");
                    let category = args
                        .get("category")
                        .and_then(Value::as_str)
                        .unwrap_or("カスタム");
                    let intro = finite_clamped(args.get("intro_sec"), 0.45, 0.0, 10.0);
                    let outro = finite_clamped(args.get("outro_sec"), 0.45, 0.0, 10.0);
                    let tpl =
                        extract_motion_template_from_scene(&scene, name, category, intro, outro);
                    let saved = folder
                        .save_motion_template(&tpl)
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "saved_path": saved.display().to_string(),
                                "template_id": tpl.id,
                                "exposed_count": tpl.exposed_params.len()
                            }).to_string()
                        }]
                    }))
                }
                "list_project_templates" => {
                    let mut list = builtin_project_templates();
                    if let Some(folder) = guard.folder.as_ref() {
                        list.extend(folder.load_project_templates());
                    }
                    let summary: Vec<Value> = list
                        .iter()
                        .map(|t| {
                            json!({
                                "id": t.id,
                                "name": t.name,
                                "description": t.description
                            })
                        })
                        .collect();
                    Ok(json!({
                        "content": [{ "type": "text", "text": json!({ "project_templates": summary }).to_string() }]
                    }))
                }
                "save_project_template" => {
                    let folder = guard
                        .folder
                        .as_ref()
                        .ok_or("project folder not initialized")?
                        .clone();
                    let name = args
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("プロジェクト雛形");
                    let tpl = ProjectTemplate {
                        id: format!("proj_custom_{}", &Uuid::new_v4().simple().to_string()[..6]),
                        name: name.to_string(),
                        description: "プロジェクトから保存された雛形".to_string(),
                        project: guard.project.clone(),
                    };
                    let saved = folder
                        .save_project_template(&tpl)
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "saved_path": saved.display().to_string(),
                                "id": tpl.id
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown manage_templates action: {other}")),
            }
        }
        "set_scene_motion_blur" => {
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or("missing action")?;
            match action {
                "get" => {
                    let scene = guard.project.active_scene().ok_or("no active scene")?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "motion_blur": scene.motion_blur,
                                "light": scene.camera.light
                            }).to_string()
                        }]
                    }))
                }
                "set_motion_blur" => {
                    let mut mb = guard
                        .project
                        .active_scene()
                        .ok_or("no active scene")?
                        .motion_blur;
                    if let Some(en) = args.get("enabled").and_then(Value::as_bool) {
                        mb.enabled = en;
                    }
                    if args.get("shutter_angle_deg").is_some() {
                        mb.shutter_angle_deg =
                            finite_clamped(args.get("shutter_angle_deg"), 180.0, 0.0, 360.0) as f32;
                    }
                    if args.get("shutter_phase_deg").is_some() {
                        mb.shutter_phase_deg =
                            finite_clamped(args.get("shutter_phase_deg"), -90.0, -180.0, 180.0)
                                as f32;
                    }
                    if let Some(ps) = args.get("preview_samples").and_then(Value::as_u64) {
                        mb.preview_samples = ps.clamp(1, 8) as u32;
                    }
                    if let Some(es) = args.get("export_samples").and_then(Value::as_u64) {
                        mb.export_samples = es.clamp(1, 16) as u32;
                    }
                    let sess = &mut *guard;
                    sess.bus
                        .execute(
                            &mut sess.project,
                            EditCommand::SetMotionBlur { motion_blur: mb },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": json!({ "motion_blur": mb }).to_string() }]
                    }))
                }
                "set_light_3d" => {
                    let mut cam = guard
                        .project
                        .active_scene()
                        .ok_or("no active scene")?
                        .camera;
                    if let Some(en) = args.get("enabled").and_then(Value::as_bool) {
                        cam.light.enabled = en;
                        if en {
                            cam.enabled = true;
                        }
                    }
                    if let Some(lt) = args.get("light_type").and_then(Value::as_str) {
                        cam.light.light_type = match lt {
                            "point" => Light3DType::Point,
                            _ => Light3DType::Directional,
                        };
                    }
                    if let Some(arr) = args.get("position").and_then(Value::as_array)
                        && arr.len() >= 3
                    {
                        cam.light.position = [
                            finite_clamped(
                                Some(&arr[0]),
                                cam.light.position[0] as f64,
                                -5000.0,
                                5000.0,
                            ) as f32,
                            finite_clamped(
                                Some(&arr[1]),
                                cam.light.position[1] as f64,
                                -5000.0,
                                5000.0,
                            ) as f32,
                            finite_clamped(
                                Some(&arr[2]),
                                cam.light.position[2] as f64,
                                -5000.0,
                                5000.0,
                            ) as f32,
                        ];
                    }
                    if args.get("intensity").is_some() {
                        cam.light.intensity =
                            finite_clamped(args.get("intensity"), 1.15, 0.0, 3.0) as f32;
                    }
                    if args.get("ambient").is_some() {
                        cam.light.ambient =
                            finite_clamped(args.get("ambient"), 0.35, 0.0, 1.5) as f32;
                    }
                    if let Some(cs) = args.get("cast_shadow").and_then(Value::as_bool) {
                        cam.light.cast_shadow = cs;
                    }
                    if args.get("shadow_softness").is_some() {
                        cam.light.shadow_softness =
                            finite_clamped(args.get("shadow_softness"), 8.0, 0.0, 24.0) as f32;
                    }
                    if args.get("shadow_opacity").is_some() {
                        cam.light.shadow_opacity =
                            finite_clamped(args.get("shadow_opacity"), 0.45, 0.0, 1.0) as f32;
                    }
                    let sess = &mut *guard;
                    sess.bus
                        .execute(
                            &mut sess.project,
                            EditCommand::SetCamera3D { camera: cam },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": json!({ "light": cam.light }).to_string() }]
                    }))
                }
                "set_particle" => {
                    let item_id = parse_uuid(args, "item_id")?;
                    let mut cfg = guard
                        .project
                        .active_scene()
                        .and_then(|s| {
                            s.layers.iter().flat_map(|l| l.items.iter()).find_map(|i| {
                                if i.id == item_id
                                    && let ItemKind::ParticleEmitter { config } = i.kind
                                {
                                    return Some(config);
                                }
                                None
                            })
                        })
                        .ok_or_else(|| format!("particle item not found: {item_id}"))?;
                    if let Some(p) = args.get("preset").and_then(Value::as_str) {
                        let preset = match p {
                            "snow" => ParticlePreset::Snow,
                            "sparks" => ParticlePreset::Sparks,
                            "light_orbs" => ParticlePreset::LightOrbs,
                            "speed_lines" => ParticlePreset::SpeedLines,
                            "action_lines" => ParticlePreset::ActionLines,
                            _ => ParticlePreset::Confetti,
                        };
                        cfg = ParticleConfig::for_preset(preset);
                    }
                    if let Some(c) = args.get("count").and_then(Value::as_u64) {
                        cfg.count = c.clamp(1, 600) as u32;
                    }
                    if let Some(sd) = args.get("seed").and_then(Value::as_u64) {
                        cfg.seed = sd;
                    }
                    if args.get("speed").is_some() {
                        cfg.speed =
                            finite_clamped(args.get("speed"), cfg.speed as f64, 0.0, 4000.0) as f32;
                    }
                    if args.get("size_px").is_some() {
                        cfg.size_px =
                            finite_clamped(args.get("size_px"), cfg.size_px as f64, 1.0, 200.0)
                                as f32;
                    }
                    let sess = &mut *guard;
                    sess.bus
                        .execute(
                            &mut sess.project,
                            EditCommand::SetParticleConfig {
                                item_id,
                                config: cfg,
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": json!({ "particle": cfg }).to_string() }]
                    }))
                }
                "set_vector_shape" => {
                    let item_id = parse_uuid(args, "item_id")?;
                    let mut cfg = guard
                        .project
                        .active_scene()
                        .and_then(|s| {
                            s.layers.iter().flat_map(|l| l.items.iter()).find_map(|i| {
                                if i.id == item_id
                                    && let ItemKind::VectorShape { config } = &i.kind
                                {
                                    return Some(config.clone());
                                }
                                None
                            })
                        })
                        .ok_or_else(|| format!("vector_shape item not found: {item_id}"))?;
                    if args.get("trim_start").is_some() {
                        cfg.trim_start =
                            finite_clamped(args.get("trim_start"), cfg.trim_start as f64, 0.0, 1.0)
                                as f32;
                    }
                    if args.get("trim_end").is_some() {
                        cfg.trim_end =
                            finite_clamped(args.get("trim_end"), cfg.trim_end as f64, 0.0, 1.0)
                                as f32;
                    }
                    if args.get("trim_offset").is_some() {
                        cfg.trim_offset = finite_clamped(
                            args.get("trim_offset"),
                            cfg.trim_offset as f64,
                            -10.0,
                            10.0,
                        ) as f32;
                    }
                    if args.get("stroke_width").is_some() {
                        cfg.stroke_width = finite_clamped(
                            args.get("stroke_width"),
                            cfg.stroke_width as f64,
                            0.0,
                            128.0,
                        ) as f32;
                    }
                    if args.get("dash_length").is_some() {
                        cfg.dash_length = finite_clamped(
                            args.get("dash_length"),
                            cfg.dash_length as f64,
                            0.0,
                            500.0,
                        ) as f32;
                    }
                    if args.get("dash_gap").is_some() {
                        cfg.dash_gap =
                            finite_clamped(args.get("dash_gap"), cfg.dash_gap as f64, 0.0, 500.0)
                                as f32;
                    }
                    let sess = &mut *guard;
                    sess.bus
                        .execute(
                            &mut sess.project,
                            EditCommand::SetVectorShape {
                                item_id,
                                config: cfg.clone(),
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": json!({ "vector_shape": cfg }).to_string() }]
                    }))
                }
                other => Err(format!("unknown set_scene_motion_blur action: {other}")),
            }
        }
        "manage_node_graph" => {
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing string argument 'action'".to_string())?;
            let s = &mut *guard;
            match action {
                "get_graph" => {
                    let scene = s
                        .project
                        .active_scene()
                        .ok_or_else(|| "no active scene".to_string())?;
                    let text = serde_json::to_string_pretty(&scene.node_graph).unwrap_or_default();
                    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
                }
                "from_layers" => {
                    let graph = {
                        let scene = s
                            .project
                            .active_scene()
                            .ok_or_else(|| "no active scene".to_string())?;
                        NodeCompositeGraph::from_scene_layers(scene)
                    };
                    let node_count = graph.nodes.len();
                    let conn_count = graph.connections.len();
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetNodeGraph { graph },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    let text = serde_json::to_string_pretty(&json!({
                        "status": "generated_from_layers",
                        "enabled": true,
                        "node_count": node_count,
                        "connection_count": conn_count,
                    }))
                    .unwrap_or_default();
                    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
                }
                "sync_to_layers" => {
                    let mut next_project = s.project.clone();
                    let summary = {
                        let scene = next_project
                            .active_scene_mut()
                            .ok_or_else(|| "no active scene".to_string())?;
                        let graph = scene.node_graph.clone();
                        sync_node_graph_to_scene_layers(&graph, scene)
                    };
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::ReplaceProjectSnapshot {
                                project: Box::new(next_project),
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    let text = serde_json::to_string_pretty(&summary).unwrap_or_default();
                    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
                }
                "set_enabled" => {
                    let enabled = args.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                    let mut graph = s
                        .project
                        .active_scene()
                        .ok_or_else(|| "no active scene".to_string())?
                        .node_graph
                        .clone();
                    graph.enabled = enabled;
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetNodeGraph { graph },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(
                        json!({ "content": [{ "type": "text", "text": json!({ "enabled": enabled }).to_string() }] }),
                    )
                }
                "add_node" => {
                    let node_type = args
                        .get("node_type")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "missing string argument 'node_type'".to_string())?;
                    let mut graph = s
                        .project
                        .active_scene()
                        .ok_or_else(|| "no active scene".to_string())?
                        .node_graph
                        .clone();
                    let kind = match node_type {
                        "layer_input" => CompositeNodeKind::LayerInput {
                            layer_index: args
                                .get("layer_index")
                                .and_then(Value::as_u64)
                                .unwrap_or(0) as usize,
                        },
                        "solid_color" => CompositeNodeKind::SolidColor {
                            color: RgbaColor::WHITE,
                        },
                        "effect" => CompositeNodeKind::Effect {
                            effect_type: args
                                .get("effect_type")
                                .and_then(Value::as_str)
                                .unwrap_or("blur")
                                .to_string(),
                            params: parse_params_map(args.get("params")),
                        },
                        "channel_split" => {
                            let ch = match args
                                .get("channel")
                                .and_then(Value::as_str)
                                .unwrap_or("luma")
                            {
                                "rgba" => ChannelSelector::Rgba,
                                "red" => ChannelSelector::Red,
                                "green" => ChannelSelector::Green,
                                "blue" => ChannelSelector::Blue,
                                "alpha" => ChannelSelector::Alpha,
                                "depth" => ChannelSelector::Depth,
                                _ => ChannelSelector::Luma,
                            };
                            CompositeNodeKind::ChannelSplit { channel: ch }
                        }
                        "channel_merge" => CompositeNodeKind::ChannelMerge,
                        "transform" => CompositeNodeKind::Transform {
                            x: finite_clamped(
                                args.get("params").and_then(|p| p.get("x")),
                                0.0,
                                -4096.0,
                                4096.0,
                            ) as f32,
                            y: finite_clamped(
                                args.get("params").and_then(|p| p.get("y")),
                                0.0,
                                -4096.0,
                                4096.0,
                            ) as f32,
                            scale_x: finite_clamped(
                                args.get("params").and_then(|p| p.get("scale_x")),
                                1.0,
                                0.05,
                                8.0,
                            ) as f32,
                            scale_y: finite_clamped(
                                args.get("params").and_then(|p| p.get("scale_y")),
                                1.0,
                                0.05,
                                8.0,
                            ) as f32,
                            rotation_deg: finite_clamped(
                                args.get("params").and_then(|p| p.get("rotation_deg")),
                                0.0,
                                -3600.0,
                                3600.0,
                            ) as f32,
                            opacity: finite_clamped(
                                args.get("params").and_then(|p| p.get("opacity")),
                                1.0,
                                0.0,
                                1.0,
                            ) as f32,
                        },
                        "merge" => {
                            let bm = match args
                                .get("blend_mode")
                                .and_then(Value::as_str)
                                .unwrap_or("normal")
                            {
                                "add" => BlendMode::Add,
                                "multiply" => BlendMode::Multiply,
                                "screen" => BlendMode::Screen,
                                _ => BlendMode::Normal,
                            };
                            CompositeNodeKind::Merge {
                                blend_mode: bm,
                                opacity: finite_clamped(
                                    args.get("params").and_then(|p| p.get("opacity")),
                                    1.0,
                                    0.0,
                                    1.0,
                                ) as f32,
                            }
                        }
                        "mask_combine" => {
                            let mode = match args
                                .get("matte_mode")
                                .and_then(Value::as_str)
                                .unwrap_or("alpha")
                            {
                                "alpha_inverted" => TrackMatteMode::AlphaInverted,
                                "luma" => TrackMatteMode::Luma,
                                "luma_inverted" => TrackMatteMode::LumaInverted,
                                _ => TrackMatteMode::Alpha,
                            };
                            CompositeNodeKind::MaskCombine { mode }
                        }
                        "color_correct" => CompositeNodeKind::ColorCorrect {
                            exposure: finite_clamped(
                                args.get("params").and_then(|p| p.get("exposure")),
                                0.0,
                                -4.0,
                                4.0,
                            ) as f32,
                            contrast: finite_clamped(
                                args.get("params").and_then(|p| p.get("contrast")),
                                1.0,
                                0.1,
                                3.0,
                            ) as f32,
                            saturation: finite_clamped(
                                args.get("params").and_then(|p| p.get("saturation")),
                                1.0,
                                0.0,
                                3.0,
                            ) as f32,
                        },
                        "depth_fog" => CompositeNodeKind::DepthFog {
                            near: finite_clamped(
                                args.get("params").and_then(|p| p.get("near")),
                                400.0,
                                0.0,
                                10000.0,
                            ) as f32,
                            far: finite_clamped(
                                args.get("params").and_then(|p| p.get("far")),
                                2000.0,
                                10.0,
                                20000.0,
                            ) as f32,
                            fog_color: RgbaColor {
                                r: 0.72,
                                g: 0.80,
                                b: 0.92,
                                a: 1.0,
                            },
                            strength: finite_clamped(
                                args.get("params").and_then(|p| p.get("strength")),
                                0.5,
                                0.0,
                                1.0,
                            ) as f32,
                        },
                        "output" => CompositeNodeKind::Output,
                        other => return Err(format!("unknown node_type: {other}")),
                    };
                    let name = args
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or(node_type);
                    let pos = [120.0 + (graph.nodes.len() as f32 * 40.0), 120.0];
                    let node_id = graph.add_node(CompositeNode::new(name, pos, kind));
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetNodeGraph { graph },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(
                        json!({ "content": [{ "type": "text", "text": json!({ "created_node_id": node_id }).to_string() }] }),
                    )
                }
                "connect" => {
                    let from_node = parse_uuid(args, "from_node")?;
                    let to_node = parse_uuid(args, "to_node")?;
                    let from_port = args
                        .get("from_port")
                        .and_then(Value::as_str)
                        .unwrap_or("out");
                    let to_port = args.get("to_port").and_then(Value::as_str).unwrap_or("in");
                    let mut graph = s
                        .project
                        .active_scene()
                        .ok_or_else(|| "no active scene".to_string())?
                        .node_graph
                        .clone();
                    graph
                        .connect(from_node, from_port, to_node, to_port)
                        .map_err(|e| e.to_string())?;
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetNodeGraph { graph },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(
                        json!({ "content": [{ "type": "text", "text": json!({ "connected": true }).to_string() }] }),
                    )
                }
                "remove_node" => {
                    let node_id = parse_uuid(args, "node_id")?;
                    let mut graph = s
                        .project
                        .active_scene()
                        .ok_or_else(|| "no active scene".to_string())?
                        .node_graph
                        .clone();
                    let removed = graph.remove_node(node_id);
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetNodeGraph { graph },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(
                        json!({ "content": [{ "type": "text", "text": json!({ "removed": removed }).to_string() }] }),
                    )
                }
                "validate" => {
                    let scene = s
                        .project
                        .active_scene()
                        .ok_or_else(|| "no active scene".to_string())?;
                    let res = scene.node_graph.topological_order();
                    let text = match res {
                        Ok(order) => json!({
                            "valid": true,
                            "enabled": scene.node_graph.enabled,
                            "topological_len": order.len(),
                            "has_output": scene.node_graph.output_node_id().is_some(),
                        })
                        .to_string(),
                        Err(e) => json!({ "valid": false, "error": e.to_string() }).to_string(),
                    };
                    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
                }
                other => Err(format!("unknown manage_node_graph action: {other}")),
            }
        }
        "vfx_stabilize_and_corner_pin" => {
            let s = &mut *guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing 'action'".to_string())?;
            let item_id = parse_uuid(args, "id")?;

            let parse_pt2 = |key: &str, fallback: [f32; 2]| -> [f32; 2] {
                args.get(key)
                    .and_then(Value::as_array)
                    .filter(|a| a.len() >= 2)
                    .and_then(|a| {
                        let x = a[0].as_f64()?;
                        let y = a[1].as_f64()?;
                        if x.is_finite() && y.is_finite() {
                            Some([x.clamp(-0.5, 1.5) as f32, y.clamp(-0.5, 1.5) as f32])
                        } else {
                            None
                        }
                    })
                    .unwrap_or(fallback)
            };

            match action {
                "analyze_and_stabilize" => {
                    let cfg = StabilizeConfig {
                        sample_interval_sec: finite_clamped(
                            args.get("sample_interval_sec"),
                            0.2,
                            0.05,
                            2.0,
                        ),
                        smoothness: finite_clamped(args.get("smoothness"), 0.78, 0.1, 0.95) as f32,
                        max_shift_ratio: finite_clamped(
                            args.get("max_shift_ratio"),
                            0.08,
                            0.01,
                            0.25,
                        ) as f32,
                        stabilize_rotation: args
                            .get("stabilize_rotation")
                            .and_then(Value::as_bool)
                            .unwrap_or(true),
                        auto_crop_zoom: args
                            .get("auto_crop_zoom")
                            .and_then(Value::as_bool)
                            .unwrap_or(true),
                    };
                    let apply = args.get("apply").and_then(Value::as_bool).unwrap_or(true);
                    let scene_id = s.project.active_scene_id;
                    let mut rg = RenderGraph::new();
                    rg.set_project_folder(s.folder.clone());
                    let samples = analyze_camera_shake(&mut rg, &s.project, scene_id, &cfg);
                    let cmds = {
                        let scene = s
                            .project
                            .active_scene()
                            .ok_or_else(|| "no active scene".to_string())?;
                        build_stabilize_commands(scene, item_id, &samples, &cfg)
                    };
                    let max_dx = samples
                        .iter()
                        .map(|sm| sm.comp_x.abs())
                        .fold(0.0f32, f32::max);
                    let max_dy = samples
                        .iter()
                        .map(|sm| sm.comp_y.abs())
                        .fold(0.0f32, f32::max);
                    let max_rot = samples
                        .iter()
                        .map(|sm| sm.comp_rot_deg.abs())
                        .fold(0.0f32, f32::max);
                    let gen_count = cmds.len();
                    let applied = if apply && !cmds.is_empty() {
                        s.bus
                            .execute(
                                &mut s.project,
                                EditCommand::Batch {
                                    label: format!("自動手ぶれ補正 ({} サンプル)", samples.len()),
                                    commands: cmds,
                                },
                                CommandSource::Mcp,
                                None,
                            )
                            .map_err(|e| e.to_string())?;
                        true
                    } else {
                        false
                    };
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "item_id": item_id,
                                "sample_count": samples.len(),
                                "max_comp_x": max_dx,
                                "max_comp_y": max_dy,
                                "max_comp_rot_deg": max_rot,
                                "generated_commands": gen_count,
                                "applied": applied
                            }).to_string()
                        }]
                    }))
                }
                "set_corner_pin" => {
                    let mut pin = s
                        .project
                        .active_scene()
                        .and_then(|sc| {
                            sc.layers
                                .iter()
                                .flat_map(|l| l.items.iter())
                                .find(|i| i.id == item_id)
                                .map(|i| i.corner_pin)
                        })
                        .ok_or_else(|| format!("item not found: {item_id}"))?;
                    if let Some(preset) = args.get("preset").and_then(Value::as_str) {
                        pin = match preset {
                            "monitor_tilt" => CornerPinConfig {
                                enabled: true,
                                tl: [0.14, 0.12],
                                tr: [0.88, 0.08],
                                br: [0.92, 0.86],
                                bl: [0.10, 0.90],
                                feather_px: 1.5,
                            },
                            "billboard_left" => CornerPinConfig {
                                enabled: true,
                                tl: [0.06, 0.08],
                                tr: [0.76, 0.18],
                                br: [0.76, 0.82],
                                bl: [0.06, 0.92],
                                feather_px: 1.5,
                            },
                            "perspective_floor" => CornerPinConfig {
                                enabled: true,
                                tl: [0.24, 0.35],
                                tr: [0.76, 0.35],
                                br: [0.96, 0.94],
                                bl: [0.04, 0.94],
                                feather_px: 2.0,
                            },
                            _ => CornerPinConfig::default(),
                        };
                    }
                    pin.enabled = args.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                    pin.tl = parse_pt2("tl", pin.tl);
                    pin.tr = parse_pt2("tr", pin.tr);
                    pin.br = parse_pt2("br", pin.br);
                    pin.bl = parse_pt2("bl", pin.bl);
                    if args.get("feather_px").is_some() {
                        pin.feather_px = finite_clamped(
                            args.get("feather_px"),
                            f64::from(pin.feather_px),
                            0.0,
                            16.0,
                        ) as f32;
                    }
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetCornerPin {
                                item_id,
                                corner_pin: pin,
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": serde_json::to_string(&pin).unwrap_or_default() }]
                    }))
                }
                "reset_corner_pin" => {
                    let pin = CornerPinConfig::default();
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetCornerPin {
                                item_id,
                                corner_pin: pin,
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": serde_json::to_string(&pin).unwrap_or_default() }]
                    }))
                }
                other => Err(format!(
                    "unknown vfx_stabilize_and_corner_pin action: {other}"
                )),
            }
        }
        "gpu_v30_pipeline" => {
            let s = &mut *guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing 'action'".to_string())?;
            match action {
                "status" => {
                    let caps = detect_gpu_v30_capabilities();
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "gpu_available": caps.gpu_available,
                                "adapter_name": caps.adapter_name,
                                "backend_name": caps.backend_name,
                                "color_shader_valid": caps.color_shader_valid,
                                "scopes_shader_valid": caps.scopes_shader_valid,
                                "composite_shader_valid": caps.composite_shader_valid
                            }).to_string()
                        }]
                    }))
                }
                "benchmark_4k" => {
                    let w = args.get("width").and_then(Value::as_u64).unwrap_or(1920) as u32;
                    let h = args.get("height").and_then(Value::as_u64).unwrap_or(1080) as u32;
                    let rep = benchmark_v30_gpu_pipeline(w, h);
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "width": rep.width,
                                "height": rep.height,
                                "executed_on_gpu": rep.executed_on_gpu,
                                "backend_name": rep.backend_name,
                                "elapsed_ms": rep.elapsed_ms,
                                "estimated_fps": rep.estimated_fps,
                                "avg_luma_ire": rep.avg_luma_ire
                            }).to_string()
                        }]
                    }))
                }
                "run_grade_and_scopes" => {
                    let t_sec = finite_clamped(
                        args.get("time_sec"),
                        s.playhead.as_seconds(),
                        0.0,
                        86_400.0,
                    );
                    let w = (args.get("width").and_then(Value::as_u64).unwrap_or(640) as u32)
                        .clamp(64, 1920);
                    let h = (args.get("height").and_then(Value::as_u64).unwrap_or(360) as u32)
                        .clamp(64, 1080);
                    let scene_id = s.project.active_scene_id;
                    let grade = s
                        .project
                        .active_scene()
                        .ok_or_else(|| "no active scene".to_string())?
                        .color_grade
                        .clone();
                    let mut rg = RenderGraph::new();
                    rg.set_project_folder(s.folder.clone());
                    let mut img = rg
                        .render_project_scene_scaled(
                            &s.project,
                            scene_id,
                            RationalTime::from_seconds(t_sec),
                            w,
                            h,
                            false,
                        )
                        .ok_or_else(|| "failed to render frame".to_string())?;
                    let (scopes, on_gpu, backend) =
                        run_gpu_or_cpu_grade_and_scopes(&mut img, &grade, None);
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "executed_on_gpu": on_gpu,
                                "backend_name": backend,
                                "avg_luma_ire": scopes.avg_luma_ire,
                                "peak_luma_ire": scopes.peak_luma_ire,
                                "clipped_highlight_pct": scopes.clipped_highlight_pct,
                                "crushed_shadow_pct": scopes.crushed_shadow_pct
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown gpu_v30_pipeline action: {other}")),
            }
        }
        "scene_3d_and_camera_track" => {
            let s = &mut *guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing 'action'".to_string())?;
            let parse_preset = |raw: Option<&str>, cur: Mesh3DPreset| match raw {
                Some("sphere") => Mesh3DPreset::Sphere,
                Some("torus") => Mesh3DPreset::Torus,
                Some("extruded_text") => Mesh3DPreset::ExtrudedText,
                Some("gltf_file") => Mesh3DPreset::GltfFile,
                Some("cube") => Mesh3DPreset::Cube,
                _ => cur,
            };
            let apply_mesh_args = |cfg: &mut Mesh3DConfig| {
                cfg.preset = parse_preset(args.get("preset").and_then(Value::as_str), cfg.preset);
                if let Some(p) = args.get("gltf_path").and_then(Value::as_str) {
                    cfg.gltf_path = if p.trim().is_empty() {
                        None
                    } else {
                        Some(p.to_string())
                    };
                }
                if let Some(txt) = args.get("text_content").and_then(Value::as_str) {
                    cfg.text_content = txt.to_string();
                }
                if args.get("extrude_depth").is_some() {
                    cfg.extrude_depth = finite_clamped(
                        args.get("extrude_depth"),
                        f64::from(cfg.extrude_depth),
                        0.05,
                        2.0,
                    ) as f32;
                }
                if let Some(hex) = args.get("base_color").and_then(Value::as_str)
                    && let Some(c) = RgbaColor::from_hex(hex)
                {
                    cfg.base_color = c;
                }
                if args.get("metallic").is_some() {
                    cfg.metallic =
                        finite_clamped(args.get("metallic"), f64::from(cfg.metallic), 0.0, 1.0)
                            as f32;
                }
                if args.get("roughness").is_some() {
                    cfg.roughness =
                        finite_clamped(args.get("roughness"), f64::from(cfg.roughness), 0.06, 1.0)
                            as f32;
                }
                if args.get("env_reflection").is_some() {
                    cfg.env_reflection = finite_clamped(
                        args.get("env_reflection"),
                        f64::from(cfg.env_reflection),
                        0.0,
                        1.0,
                    ) as f32;
                }
                if let Some(wf) = args.get("wireframe").and_then(Value::as_bool) {
                    cfg.wireframe = wf;
                }
                if args.get("auto_spin_deg_per_sec").is_some() {
                    cfg.auto_spin_deg_per_sec = finite_clamped(
                        args.get("auto_spin_deg_per_sec"),
                        f64::from(cfg.auto_spin_deg_per_sec),
                        -720.0,
                        720.0,
                    ) as f32;
                }
            };

            match action {
                "add_mesh_3d" => {
                    let max_layer = s
                        .project
                        .active_scene()
                        .map(|sc| sc.layers.len().saturating_sub(1))
                        .ok_or_else(|| "no active scene".to_string())?;
                    let layer_index = (args.get("layer_index").and_then(Value::as_u64).unwrap_or(0)
                        as usize)
                        .min(max_layer);
                    let name = args
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("3D Mesh")
                        .to_string();
                    let start = RationalTime::from_seconds(finite_clamped(
                        args.get("start_sec"),
                        s.playhead.as_seconds(),
                        0.0,
                        86_400.0,
                    ));
                    let dur = RationalTime::from_seconds(finite_clamped(
                        args.get("duration_sec"),
                        4.0,
                        0.1,
                        3600.0,
                    ));
                    let mut cfg = Mesh3DConfig::default();
                    apply_mesh_args(&mut cfg);
                    let item = TimelineItem::new(
                        name,
                        start,
                        dur,
                        ItemKind::Mesh3D {
                            config: cfg.clone(),
                        },
                    );
                    let created_id = item.id;
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::AddObject { layer_index, item },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": json!({ "created_id": created_id, "mesh_3d": cfg }).to_string() }]
                    }))
                }
                "update_mesh_3d" => {
                    let item_id = parse_uuid(args, "id")?;
                    let mut cfg = s
                        .project
                        .active_scene()
                        .and_then(|sc| {
                            sc.layers.iter().flat_map(|l| l.items.iter()).find_map(|i| {
                                if i.id == item_id
                                    && let ItemKind::Mesh3D { config } = &i.kind
                                {
                                    Some(config.clone())
                                } else {
                                    None
                                }
                            })
                        })
                        .ok_or_else(|| format!("mesh_3d item not found: {item_id}"))?;
                    apply_mesh_args(&mut cfg);
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetMesh3D {
                                item_id,
                                config: cfg.clone(),
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": json!({ "item_id": item_id, "mesh_3d": cfg }).to_string() }]
                    }))
                }
                "emit_sample_glb" => {
                    let cube = build_cube_mesh();
                    let bytes = encode_mesh_to_glb(&cube);
                    let parsed = parse_gltf_or_glb(&bytes, None)?;
                    let out_dir = s
                        .folder
                        .as_ref()
                        .map(|f| f.assets_dir())
                        .unwrap_or_else(|| std::env::temp_dir().join("ve_glb"));
                    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
                    let glb_path = out_dir.join("sample_cube.glb");
                    std::fs::write(&glb_path, &bytes).map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "glb_path": glb_path.display().to_string(),
                                "bytes": bytes.len(),
                                "triangles": parsed.triangle_count(),
                                "vertices": parsed.vertices.len()
                            }).to_string()
                        }]
                    }))
                }
                "track_3d_camera" => {
                    let step = finite_clamped(args.get("sample_interval_sec"), 0.25, 0.1, 2.0);
                    let apply = args.get("apply").and_then(Value::as_bool).unwrap_or(true);
                    let scene_id = s.project.active_scene_id;
                    let mut rg = RenderGraph::new();
                    rg.set_project_folder(s.folder.clone());
                    let samples = analyze_3d_camera_track(&mut rg, &s.project, scene_id, step);
                    let base_cam = s
                        .project
                        .active_scene()
                        .map(|sc| sc.camera)
                        .ok_or_else(|| "no active scene".to_string())?;
                    let applied = if apply
                        && let Some(cmd) = build_3d_camera_track_command(&base_cam, &samples)
                    {
                        s.bus
                            .execute(&mut s.project, cmd, CommandSource::Mcp, None)
                            .map_err(|e| e.to_string())?;
                        true
                    } else {
                        false
                    };
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "sample_count": samples.len(),
                                "applied": applied,
                                "camera_position": s.project.active_scene().map(|sc| sc.camera.position)
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown scene_3d_and_camera_track action: {other}")),
            }
        }
        "physics_simulate_and_bake" => {
            let s = &mut *guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing 'action'".to_string())?;
            let preset = match args.get("preset").and_then(Value::as_str) {
                Some("domino_cascade") => PhysicsPreset::DominoCascade,
                Some("zero_gravity_float") => PhysicsPreset::ZeroGravityFloat,
                Some("orbital_vortex") => PhysicsPreset::OrbitalVortex,
                _ => PhysicsPreset::BouncingTitle,
            };
            let base = preset.default_config();
            let cfg = PhysicsConfig {
                preset,
                gravity_y: finite_clamped(
                    args.get("gravity_y"),
                    f64::from(base.gravity_y),
                    -4000.0,
                    4000.0,
                ) as f32,
                restitution: finite_clamped(
                    args.get("restitution"),
                    f64::from(base.restitution),
                    0.0,
                    1.0,
                ) as f32,
                friction: finite_clamped(args.get("friction"), f64::from(base.friction), 0.5, 1.0)
                    as f32,
                air_drag: finite_clamped(args.get("air_drag"), f64::from(base.air_drag), 0.0, 0.5)
                    as f32,
                initial_vx: finite_clamped(
                    args.get("initial_vx"),
                    f64::from(base.initial_vx),
                    -3000.0,
                    3000.0,
                ) as f32,
                initial_vy: finite_clamped(
                    args.get("initial_vy"),
                    f64::from(base.initial_vy),
                    -3000.0,
                    3000.0,
                ) as f32,
                initial_spin_deg: finite_clamped(
                    args.get("initial_spin_deg"),
                    f64::from(base.initial_spin_deg),
                    -720.0,
                    720.0,
                ) as f32,
                central_attract: finite_clamped(
                    args.get("central_attract"),
                    f64::from(base.central_attract),
                    0.0,
                    3000.0,
                ) as f32,
                duration_sec: finite_clamped(
                    args.get("duration_sec"),
                    base.duration_sec,
                    0.2,
                    30.0,
                ),
                bake_step_frames: args
                    .get("bake_step_frames")
                    .and_then(Value::as_u64)
                    .map(|v| v.clamp(1, 30) as u32)
                    .unwrap_or(base.bake_step_frames),
                floor_ratio: finite_clamped(
                    args.get("floor_ratio"),
                    f64::from(base.floor_ratio),
                    0.2,
                    0.98,
                ) as f32,
            };

            let target_ids: Vec<Uuid> = {
                let sc = s
                    .project
                    .active_scene()
                    .ok_or_else(|| "no active scene".to_string())?;
                if let Some(arr) = args.get("item_ids").and_then(Value::as_array)
                    && !arr.is_empty()
                {
                    arr.iter()
                        .filter_map(|v| v.as_str().and_then(|st| Uuid::parse_str(st).ok()))
                        .collect()
                } else {
                    sc.layers
                        .iter()
                        .flat_map(|l| l.items.iter())
                        .filter(|i| {
                            !matches!(
                                i.kind,
                                ItemKind::Audio { .. }
                                    | ItemKind::FilterObject { .. }
                                    | ItemKind::Transition { .. }
                                    | ItemKind::GroupControl { .. }
                                    | ItemKind::GenerativeBg { .. }
                            )
                        })
                        .take(12)
                        .map(|i| i.id)
                        .collect()
                }
            };

            if target_ids.is_empty() {
                return Err("物理演算の対象オブジェクトが見つかりません".into());
            }

            match action {
                "preview_trajectory" => {
                    let sc = s
                        .project
                        .active_scene()
                        .ok_or_else(|| "no active scene".to_string())?;
                    let trajectories = simulate_scene_physics(sc, &target_ids, &cfg);
                    let total_bounces: u32 = trajectories.iter().map(|t| t.bounce_count).sum();
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "body_count": trajectories.len(),
                                "total_bounces": total_bounces,
                                "trajectories": trajectories
                            }).to_string()
                        }]
                    }))
                }
                "bake_keyframes" => {
                    let (trajectories, cmds) = {
                        let sc = s
                            .project
                            .active_scene()
                            .ok_or_else(|| "no active scene".to_string())?;
                        build_physics_bake_commands(sc, &target_ids, &cfg)
                    };
                    let cmd_count = cmds.len();
                    let total_bounces: u32 = trajectories.iter().map(|t| t.bounce_count).sum();
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::Batch {
                                label: format!("2D 剛体物理ベイク ({}体)", trajectories.len()),
                                commands: cmds,
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "baked": true,
                                "body_count": trajectories.len(),
                                "keyframe_commands": cmd_count,
                                "total_bounces": total_bounces
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown physics_simulate_and_bake action: {other}")),
            }
        }
        "generative_bg_and_foley" => {
            let s = &mut *guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing 'action'".to_string())?;
            let parse_bg_preset = |raw: Option<&str>| -> GenerativeBgPreset {
                match raw {
                    Some("cinema_bokeh") => GenerativeBgPreset::CinemaBokeh,
                    Some("cyber_grid3d") | Some("cyber_grid_3d") => GenerativeBgPreset::CyberGrid3D,
                    Some("liquid_marble") => GenerativeBgPreset::LiquidMarble,
                    Some("starfield_warp") => GenerativeBgPreset::StarfieldWarp,
                    Some("abstract_waves") => GenerativeBgPreset::AbstractWaves,
                    _ => GenerativeBgPreset::AuroraBorealis,
                }
            };

            match action {
                "add_generative_bg" => {
                    let (sc_w, sc_h, layer_count) = {
                        let sc = s
                            .project
                            .active_scene()
                            .ok_or_else(|| "no active scene".to_string())?;
                        (sc.width as f32, sc.height as f32, sc.layers.len())
                    };
                    let preset = parse_bg_preset(args.get("preset").and_then(Value::as_str));
                    let base = GenerativeBgPreset::for_preset(preset);
                    let cfg = GenerativeBgConfig {
                        preset,
                        seed: args
                            .get("seed")
                            .and_then(Value::as_u64)
                            .unwrap_or(base.seed),
                        speed: finite_clamped(args.get("speed"), f64::from(base.speed), 0.0, 10.0)
                            as f32,
                        scale: finite_clamped(args.get("scale"), f64::from(base.scale), 0.1, 10.0)
                            as f32,
                        complexity: args
                            .get("complexity")
                            .and_then(Value::as_u64)
                            .map(|v| v.clamp(1, 8) as u32)
                            .unwrap_or(base.complexity),
                        width: finite_clamped(args.get("width"), f64::from(sc_w), 32.0, 8192.0)
                            as f32,
                        height: finite_clamped(args.get("height"), f64::from(sc_h), 32.0, 8192.0)
                            as f32,
                        ..base
                    };
                    let layer_index = args
                        .get("layer_index")
                        .and_then(Value::as_u64)
                        .map(|v| (v as usize).min(layer_count.saturating_sub(1)))
                        .unwrap_or(0);
                    let start = RationalTime::from_seconds(finite_clamped(
                        args.get("start_sec"),
                        0.0,
                        0.0,
                        36000.0,
                    ));
                    let dur = RationalTime::from_seconds(finite_clamped(
                        args.get("duration_sec"),
                        5.0,
                        0.1,
                        3600.0,
                    ));
                    let name = args
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or(preset.display_name())
                        .to_string();
                    let item =
                        TimelineItem::new(name, start, dur, ItemKind::GenerativeBg { config: cfg });
                    let item_id = item.id;
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::AddObject { layer_index, item },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({ "item_id": item_id.to_string(), "preset": preset, "config": cfg }).to_string()
                        }]
                    }))
                }
                "update_generative_bg" => {
                    let id_str = args
                        .get("id")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "missing 'id'".to_string())?;
                    let item_id = Uuid::parse_str(id_str).map_err(|e| e.to_string())?;
                    let cur = {
                        let sc = s
                            .project
                            .active_scene()
                            .ok_or_else(|| "no active scene".to_string())?;
                        sc.layers
                            .iter()
                            .flat_map(|l| l.items.iter())
                            .find_map(|i| {
                                if i.id == item_id
                                    && let ItemKind::GenerativeBg { config } = i.kind
                                {
                                    Some(config)
                                } else {
                                    None
                                }
                            })
                            .ok_or_else(|| format!("GenerativeBg item not found: {item_id}"))?
                    };
                    let next_preset = if args.get("preset").is_some() {
                        parse_bg_preset(args.get("preset").and_then(Value::as_str))
                    } else {
                        cur.preset
                    };
                    let base = if next_preset != cur.preset {
                        let mut p = GenerativeBgPreset::for_preset(next_preset);
                        p.width = cur.width;
                        p.height = cur.height;
                        p
                    } else {
                        cur
                    };
                    let next = GenerativeBgConfig {
                        preset: next_preset,
                        seed: args
                            .get("seed")
                            .and_then(Value::as_u64)
                            .unwrap_or(base.seed),
                        speed: finite_clamped(args.get("speed"), f64::from(base.speed), 0.0, 10.0)
                            as f32,
                        scale: finite_clamped(args.get("scale"), f64::from(base.scale), 0.1, 10.0)
                            as f32,
                        complexity: args
                            .get("complexity")
                            .and_then(Value::as_u64)
                            .map(|v| v.clamp(1, 8) as u32)
                            .unwrap_or(base.complexity),
                        width: finite_clamped(
                            args.get("width"),
                            f64::from(base.width),
                            32.0,
                            8192.0,
                        ) as f32,
                        height: finite_clamped(
                            args.get("height"),
                            f64::from(base.height),
                            32.0,
                            8192.0,
                        ) as f32,
                        ..base
                    };
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetGenerativeBg {
                                item_id,
                                config: next,
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({ "item_id": item_id.to_string(), "config": next }).to_string()
                        }]
                    }))
                }
                "synthesize_foley_sfx" => {
                    let preset = FoleyPreset::from_id(
                        args.get("preset")
                            .and_then(Value::as_str)
                            .unwrap_or("whoosh_transition"),
                    );
                    let dur_sec = finite_clamped(
                        args.get("duration_sec"),
                        f64::from(preset.default_duration_sec()),
                        0.08,
                        30.0,
                    ) as f32;
                    let seed = args.get("seed").and_then(Value::as_u64).unwrap_or(40_2026);
                    let start = RationalTime::from_seconds(finite_clamped(
                        args.get("start_sec"),
                        0.0,
                        0.0,
                        36000.0,
                    ));
                    let apply = args.get("apply").and_then(Value::as_bool).unwrap_or(true);
                    let sr = s.project.settings.audio_sample_rate.max(8_000);
                    let pcm = synthesize_foley_sfx_pcm(preset, dur_sec, sr, seed);

                    let out_dir = s
                        .folder
                        .as_ref()
                        .map(|f| f.assets_dir().join("foley"))
                        .unwrap_or_else(|| std::env::temp_dir().join("ve_foley"));
                    let wav_name = format!("foley_{}_{seed}.wav", preset.id());
                    let wav_path = out_dir.join(&wav_name);
                    write_stereo_wav_file(&wav_path, &pcm, sr)?;

                    let asset_id = Uuid::new_v4();
                    s.project.assets.push(AssetRef {
                        id: asset_id,
                        name: wav_name.clone(),
                        relative_path: Some(PathBuf::from("assets/foley").join(&wav_name)),
                        fallback_absolute_path: Some(wav_path.clone()),
                        collected: true,
                        kind: MediaKind::Audio,
                    });

                    let mut placed_item_id: Option<String> = None;
                    if apply {
                        let layer_count = s
                            .project
                            .active_scene()
                            .map(|sc| sc.layers.len())
                            .ok_or_else(|| "no active scene".to_string())?;
                        let audio_layer = args
                            .get("audio_layer_index")
                            .and_then(Value::as_u64)
                            .map(|v| (v as usize).min(layer_count.saturating_sub(1)))
                            .unwrap_or_else(|| layer_count.saturating_sub(1));
                        let clip = TimelineItem::new(
                            format!("SFX: {}", preset.display_name()),
                            start,
                            RationalTime::from_seconds(f64::from(dur_sec)),
                            ItemKind::Audio {
                                asset_id,
                                source_offset: RationalTime::ZERO,
                                volume: 0.9,
                            },
                        );
                        placed_item_id = Some(clip.id.to_string());
                        s.bus
                            .execute(
                                &mut s.project,
                                EditCommand::AddObject {
                                    layer_index: audio_layer,
                                    item: clip,
                                },
                                CommandSource::Mcp,
                                None,
                            )
                            .map_err(|e| e.to_string())?;
                    }

                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "preset": preset.id(),
                                "wav_path": wav_path.display().to_string(),
                                "asset_id": asset_id.to_string(),
                                "item_id": placed_item_id,
                                "duration_sec": dur_sec,
                                "sample_rate": sr,
                                "applied": apply
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown generative_bg_and_foley action: {other}")),
            }
        }
        "rig2d_puppet_control" => {
            let s = &mut *guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing 'action'".to_string())?;
            let parse_rig_preset =
                |raw: Option<&str>, fallback: RigMotionPreset| -> RigMotionPreset {
                    match raw {
                        Some("idle_breathe") => RigMotionPreset::IdleBreathe,
                        Some("wave_hello") => RigMotionPreset::WaveHello,
                        Some("nod_talk") => RigMotionPreset::NodTalk,
                        Some("dance_bounce") => RigMotionPreset::DanceBounce,
                        _ => fallback,
                    }
                };
            let apply_rig_overrides = |mut cfg: Rig2DConfig| -> Rig2DConfig {
                cfg.preset =
                    parse_rig_preset(args.get("preset").and_then(Value::as_str), cfg.preset);
                cfg.motion_speed = finite_clamped(
                    args.get("motion_speed"),
                    f64::from(cfg.motion_speed),
                    0.0,
                    6.0,
                ) as f32;
                cfg.motion_amp =
                    finite_clamped(args.get("motion_amp"), f64::from(cfg.motion_amp), 0.0, 3.0)
                        as f32;
                if let Some(ov) = args.get("show_bones_overlay").and_then(Value::as_bool) {
                    cfg.show_bones_overlay = ov;
                }
                if let Some(en) = args.get("right_ik_enabled").and_then(Value::as_bool) {
                    cfg.right_arm_ik.enabled = en;
                }
                if let Some(x) = args.get("right_ik_x").and_then(Value::as_f64) {
                    cfg.right_arm_ik.target_pos[0] = x.clamp(-400.0, 400.0) as f32;
                }
                if let Some(y) = args.get("right_ik_y").and_then(Value::as_f64) {
                    cfg.right_arm_ik.target_pos[1] = y.clamp(-400.0, 400.0) as f32;
                }
                if let Some(en) = args.get("left_ik_enabled").and_then(Value::as_bool) {
                    cfg.left_arm_ik.enabled = en;
                }
                if let Some(x) = args.get("left_ik_x").and_then(Value::as_f64) {
                    cfg.left_arm_ik.target_pos[0] = x.clamp(-400.0, 400.0) as f32;
                }
                if let Some(y) = args.get("left_ik_y").and_then(Value::as_f64) {
                    cfg.left_arm_ik.target_pos[1] = y.clamp(-400.0, 400.0) as f32;
                }
                cfg
            };

            match action {
                "add_rigged_puppet" => {
                    let layer_count = s
                        .project
                        .active_scene()
                        .map(|sc| sc.layers.len())
                        .ok_or_else(|| "no active scene".to_string())?;
                    let layer_index = args
                        .get("layer_index")
                        .and_then(Value::as_u64)
                        .map(|v| (v as usize).min(layer_count.saturating_sub(1)))
                        .unwrap_or(1.min(layer_count.saturating_sub(1)));
                    let start = RationalTime::from_seconds(finite_clamped(
                        args.get("start_sec"),
                        0.0,
                        0.0,
                        36000.0,
                    ));
                    let dur = RationalTime::from_seconds(finite_clamped(
                        args.get("duration_sec"),
                        5.0,
                        0.1,
                        3600.0,
                    ));
                    let cfg = apply_rig_overrides(Rig2DConfig::default_humanoid());
                    let name = args
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("Rig2D Puppet")
                        .to_string();
                    let item = TimelineItem::new(
                        name,
                        start,
                        dur,
                        ItemKind::RiggedPuppet {
                            config: Box::new(cfg.clone()),
                        },
                    );
                    let item_id = item.id;
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::AddObject { layer_index, item },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "item_id": item_id.to_string(),
                                "bone_count": cfg.bones.len(),
                                "preset": cfg.preset
                            }).to_string()
                        }]
                    }))
                }
                "update_rig" => {
                    let id_str = args
                        .get("id")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "missing 'id'".to_string())?;
                    let item_id = Uuid::parse_str(id_str).map_err(|e| e.to_string())?;
                    let cur = {
                        let sc = s
                            .project
                            .active_scene()
                            .ok_or_else(|| "no active scene".to_string())?;
                        sc.layers
                            .iter()
                            .flat_map(|l| l.items.iter())
                            .find_map(|i| {
                                if i.id == item_id
                                    && let ItemKind::RiggedPuppet { config } = &i.kind
                                {
                                    Some((**config).clone())
                                } else {
                                    None
                                }
                            })
                            .ok_or_else(|| format!("RiggedPuppet not found: {item_id}"))?
                    };
                    let next = apply_rig_overrides(cur);
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetRig2D {
                                item_id,
                                config: Box::new(next.clone()),
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "item_id": item_id.to_string(),
                                "preset": next.preset,
                                "right_arm_ik": next.right_arm_ik
                            }).to_string()
                        }]
                    }))
                }
                "solve_ik_preview" => {
                    let t = finite_clamped(args.get("time_sec"), 0.0, 0.0, 3600.0) as f32;
                    let cfg = apply_rig_overrides(Rig2DConfig::default_humanoid());
                    let poses = evaluate_rig_poses(&cfg, t);
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "time_sec": t,
                                "bone_count": poses.len(),
                                "poses": poses
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown rig2d_puppet_control action: {other}")),
            }
        }
        "data_chart_motion_graphics" => {
            let s = &mut *guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or("create_chart");
            let parse_kind = |k: &str| match k {
                "line_chart" => ChartKind::LineChart,
                "donut_chart" => ChartKind::DonutChart,
                "kpi_counter" => ChartKind::KpiCounter,
                _ => ChartKind::BarChart,
            };
            let parse_points_from_args = |a: &Value| -> Option<Vec<DataSeriesPoint>> {
                if let Some(csv) = a.get("csv_text").and_then(Value::as_str) {
                    let parsed = parse_chart_csv(csv);
                    if !parsed.is_empty() {
                        return Some(parsed);
                    }
                }
                if let Some(arr) = a.get("points").and_then(Value::as_array) {
                    let mut pts = Vec::new();
                    for item in arr {
                        let label = item
                            .get("label")
                            .and_then(Value::as_str)
                            .unwrap_or("ITEM")
                            .to_string();
                        let val = item.get("value").and_then(Value::as_f64).unwrap_or(0.0);
                        let color = item
                            .get("color")
                            .and_then(Value::as_str)
                            .and_then(RgbaColor::from_hex);
                        if val.is_finite() {
                            pts.push(DataSeriesPoint {
                                label,
                                value: val,
                                color,
                            });
                        }
                    }
                    if !pts.is_empty() {
                        return Some(pts);
                    }
                }
                None
            };
            let apply_cfg_overrides = |cfg: &mut DataChartConfig, a: &Value| {
                if let Some(k) = a.get("kind").and_then(Value::as_str) {
                    cfg.kind = parse_kind(k);
                }
                if let Some(t) = a.get("title").and_then(Value::as_str) {
                    cfg.title = t.to_string();
                }
                if let Some(u) = a.get("unit_suffix").and_then(Value::as_str) {
                    cfg.unit_suffix = u.to_string();
                }
                if let Some(pts) = parse_points_from_args(a) {
                    cfg.points = pts;
                }
                if let Some(sec) = a.get("animate_in_sec").and_then(Value::as_f64) {
                    cfg.animate_in_sec = (sec as f32).clamp(0.05, 30.0);
                }
                if let Some(w) = a.get("width").and_then(Value::as_f64) {
                    cfg.width = (w as f32).clamp(120.0, 4096.0);
                }
                if let Some(h) = a.get("height").and_then(Value::as_f64) {
                    cfg.height = (h as f32).clamp(80.0, 4096.0);
                }
                if let Some(hex) = a.get("accent_color").and_then(Value::as_str)
                    && let Some(c) = RgbaColor::from_hex(hex)
                {
                    cfg.accent_color = c;
                }
                if let Some(hex) = a.get("bg_color").and_then(Value::as_str)
                    && let Some(c) = RgbaColor::from_hex(hex)
                {
                    cfg.bg_color = c;
                }
                if let Some(g) = a.get("show_grid").and_then(Value::as_bool) {
                    cfg.show_grid = g;
                }
                if let Some(v) = a.get("show_values").and_then(Value::as_bool) {
                    cfg.show_values = v;
                }
            };

            match action {
                "create_chart" => {
                    let kind = args
                        .get("kind")
                        .and_then(Value::as_str)
                        .map(parse_kind)
                        .unwrap_or(ChartKind::BarChart);
                    let mut cfg = DataChartConfig::for_kind(kind);
                    apply_cfg_overrides(&mut cfg, args);

                    let scene = s.project.active_scene().ok_or("no active scene")?;
                    let layer_index = args
                        .get("layer_index")
                        .and_then(Value::as_u64)
                        .map(|v| v as usize)
                        .unwrap_or(1)
                        .min(scene.layers.len().saturating_sub(1));
                    let start = RationalTime::from_seconds(finite_clamped(
                        args.get("start_sec"),
                        0.0,
                        0.0,
                        36000.0,
                    ));
                    let duration = RationalTime::from_seconds(finite_clamped(
                        args.get("duration_sec"),
                        5.0,
                        0.1,
                        3600.0,
                    ));
                    let name = args
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or(&cfg.title)
                        .to_string();
                    let default_x = ((scene.width as f32) - cfg.width) * 0.5;
                    let default_y = ((scene.height as f32) - cfg.height) * 0.5;
                    let mut item = TimelineItem::new(
                        name,
                        start,
                        duration,
                        ItemKind::DataChart {
                            config: Box::new(cfg.clone()),
                        },
                    );
                    item.transform.x = args
                        .get("x")
                        .and_then(Value::as_f64)
                        .map(|v| v as f32)
                        .unwrap_or(default_x);
                    item.transform.y = args
                        .get("y")
                        .and_then(Value::as_f64)
                        .map(|v| v as f32)
                        .unwrap_or(default_y);
                    let created_id = item.id;
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::AddObject { layer_index, item },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "created_id": created_id,
                                "layer_index": layer_index,
                                "point_count": cfg.points.len(),
                                "config": cfg
                            }).to_string()
                        }]
                    }))
                }
                "update_chart" | "import_csv" => {
                    let item_id = args
                        .get("item_id")
                        .and_then(Value::as_str)
                        .and_then(|u| Uuid::parse_str(u).ok())
                        .ok_or("missing or invalid 'item_id'")?;
                    let existing_cfg = {
                        let scene = s.project.active_scene().ok_or("no active scene")?;
                        scene
                            .layers
                            .iter()
                            .flat_map(|l| l.items.iter())
                            .find(|i| i.id == item_id)
                            .and_then(|i| match &i.kind {
                                ItemKind::DataChart { config } => Some((**config).clone()),
                                _ => None,
                            })
                            .ok_or("DataChart item not found")?
                    };
                    let mut next = existing_cfg;
                    apply_cfg_overrides(&mut next, args);
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetDataChart {
                                item_id,
                                config: Box::new(next.clone()),
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "updated": true,
                                "item_id": item_id,
                                "point_count": next.points.len(),
                                "config": next
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!(
                    "unknown data_chart_motion_graphics action: {other}"
                )),
            }
        }
        "vfx_paint_and_inpaint" => {
            let s = &mut *guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or("add_stroke");
            let item_id = args
                .get("item_id")
                .and_then(Value::as_str)
                .and_then(|u| Uuid::parse_str(u).ok())
                .ok_or("missing or invalid 'item_id'")?;
            let current = {
                let scene = s.project.active_scene().ok_or("no active scene")?;
                scene
                    .layers
                    .iter()
                    .flat_map(|l| l.items.iter())
                    .find(|i| i.id == item_id)
                    .map(|i| i.vfx_paint.clone())
                    .ok_or("item not found")?
            };

            match action {
                "get_paint" => Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": json!({
                            "item_id": item_id,
                            "stroke_count": current.strokes.len(),
                            "vfx_paint": current
                        }).to_string()
                    }]
                })),
                "clear_paint" => {
                    let cleared = VfxPaintConfig::default();
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetVfxPaint {
                                item_id,
                                vfx_paint: cleared.clone(),
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({ "cleared": true, "item_id": item_id }).to_string()
                        }]
                    }))
                }
                "add_stroke" | "set_paint" => {
                    let tool = match args
                        .get("tool")
                        .and_then(Value::as_str)
                        .unwrap_or("brush_glow")
                    {
                        "clone_stamp" => PaintToolKind::CloneStamp,
                        "wire_erase_inpaint" => PaintToolKind::WireEraseInpaint,
                        "blur_redact" => PaintToolKind::BlurRedact,
                        _ => PaintToolKind::BrushGlow,
                    };
                    let mut pts = Vec::new();
                    if let Some(arr) = args.get("points").and_then(Value::as_array) {
                        for p in arr {
                            if let Some(pair) = p.as_array()
                                && pair.len() >= 2
                                && let (Some(x), Some(y)) = (pair[0].as_f64(), pair[1].as_f64())
                            {
                                pts.push([(x as f32).clamp(0.0, 1.0), (y as f32).clamp(0.0, 1.0)]);
                            }
                        }
                    }
                    if pts.is_empty() {
                        pts = vec![[0.2, 0.5], [0.8, 0.5]];
                    }
                    let radius_px = finite_clamped(args.get("radius_px"), 14.0, 1.0, 256.0) as f32;
                    let hardness = finite_clamped(args.get("hardness"), 0.72, 0.05, 1.0) as f32;
                    let opacity = finite_clamped(args.get("opacity"), 1.0, 0.0, 1.0) as f32;
                    let write_on_progress =
                        finite_clamped(args.get("write_on_progress"), 1.0, 0.0, 1.0) as f32;
                    let color = args
                        .get("color")
                        .and_then(Value::as_str)
                        .and_then(RgbaColor::from_hex)
                        .unwrap_or(RgbaColor {
                            r: 1.0,
                            g: 0.28,
                            b: 0.52,
                            a: 1.0,
                        });
                    let clone_offset = args
                        .get("clone_offset")
                        .and_then(Value::as_array)
                        .and_then(|a| {
                            if a.len() >= 2 {
                                Some([
                                    a[0].as_f64().unwrap_or(0.08) as f32,
                                    a[1].as_f64().unwrap_or(0.0) as f32,
                                ])
                            } else {
                                None
                            }
                        })
                        .unwrap_or([0.08, 0.0]);

                    let stroke = PaintStroke {
                        id: Uuid::new_v4(),
                        tool,
                        points: pts,
                        radius_px,
                        hardness,
                        opacity,
                        color,
                        clone_offset,
                        write_on_progress,
                    };

                    let mut next = if action == "set_paint" {
                        VfxPaintConfig {
                            enabled: args.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                            animate_write_on: args
                                .get("animate_write_on")
                                .and_then(Value::as_bool)
                                .unwrap_or(false),
                            strokes: vec![stroke],
                        }
                    } else {
                        let mut c = current;
                        c.enabled = args.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                        if let Some(aw) = args.get("animate_write_on").and_then(Value::as_bool) {
                            c.animate_write_on = aw;
                        }
                        c.strokes.push(stroke);
                        c
                    };
                    if let Some(en) = args.get("enabled").and_then(Value::as_bool) {
                        next.enabled = en;
                    }
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetVfxPaint {
                                item_id,
                                vfx_paint: next.clone(),
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "applied": true,
                                "item_id": item_id,
                                "stroke_count": next.strokes.len(),
                                "vfx_paint": next
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown vfx_paint_and_inpaint action: {other}")),
            }
        }
        other => Err(format!("unknown tool: {other}")),
    }
}
