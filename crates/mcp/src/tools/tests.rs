//! 内蔵 MCP 統合テストスイート（v0.3〜v7.0 全 82 ツール）

use super::*;
use crate::session::EditorSession;
use std::sync::{Arc, Mutex};
use ve_core::{Project, ProjectSettings};

#[test]
fn v03_mcp_tools_align_search_replace_layer_audio_and_preset() {
    let mut proj = Project::new("McpV03", ProjectSettings::default());
    let t1 = TimelineItem::new(
        "Sub1",
        RationalTime::from_seconds(0.0),
        RationalTime::from_seconds(1.5),
        ItemKind::Text {
            content: "誤字テスト1".into(),
            font_family: "sans-serif".into(),
            font_size: 40.0,
            color: RgbaColor::WHITE,
            animator: TextAnimator::default(),
        },
    );
    let t2 = TimelineItem::new(
        "Sub2",
        RationalTime::from_seconds(5.0),
        RationalTime::from_seconds(1.5),
        ItemKind::Text {
            content: "誤字テスト2".into(),
            font_family: "sans-serif".into(),
            font_size: 40.0,
            color: RgbaColor::WHITE,
            animator: TextAnimator::default(),
        },
    );
    let (id1, id2) = (t1.id, t2.id);
    proj.active_scene_mut().unwrap().layers[1]
        .items
        .extend([t1, t2]);
    let session = Arc::new(Mutex::new(EditorSession::new(proj)));

    call_tool(
        &session,
        "align_items",
        &json!({"mode": "close_gaps", "ids": [id1.to_string(), id2.to_string()], "gap_sec": 0.5}),
    )
    .unwrap();
    {
        let g = session.lock().unwrap();
        assert!(
            (g.project.active_scene().unwrap().layers[1].items[1]
                .start
                .as_seconds()
                - 2.0)
                .abs()
                < 1e-6
        );
    }

    call_tool(
        &session,
        "search_replace_text",
        &json!({"query": "誤字", "replacement": "修正済み"}),
    )
    .unwrap();
    call_tool(
        &session,
        "set_layer_settings",
        &json!({"layer_index": 1, "offset_x": 20.0, "volume": 0.8, "color_hex": "#33aa55"}),
    )
    .unwrap();
    call_tool(
        &session,
        "add_audio_effect",
        &json!({"effect_type": "limiter", "target_lufs": -14.0}),
    )
    .unwrap();
    call_tool(
        &session,
        "apply_motion_preset",
        &json!({"id": id1.to_string(), "preset": "slide_in_left"}),
    )
    .unwrap();

    let g = session.lock().unwrap();
    let scene = g.project.active_scene().unwrap();
    assert_eq!(scene.target_lufs, Some(-14.0));
    assert_eq!(scene.master_audio_effects.len(), 1);
    assert!((scene.layers[1].offset_x - 20.0).abs() < 1e-4);
    assert!(scene.layers[1].items[0].keyframes.contains_key("x"));
    if let ItemKind::Text { content, .. } = &scene.layers[1].items[0].kind {
        assert_eq!(content, "修正済みテスト1");
    } else {
        panic!("expected text");
    }
}

#[test]
fn v04_mcp_tools_markers_refine_wrap_and_guarded_export() {
    let mut proj = Project::new("McpV04", ProjectSettings::default());
    proj.active_scene_mut().unwrap().layers[1]
        .items
        .push(TimelineItem::new(
            "LongSub",
            RationalTime::ZERO,
            RationalTime::from_seconds(2.0),
            ItemKind::Text {
                content: "これは非常に長い字幕テキストなので自動で読みやすく改行されます。".into(),
                font_family: "sans-serif".into(),
                font_size: 40.0,
                color: RgbaColor::WHITE,
                animator: TextAnimator::default(),
            },
        ));
    let session = Arc::new(Mutex::new(EditorSession::new(proj)));

    call_tool(
        &session,
        "manage_markers",
        &json!({"action": "add", "time_sec": 12.0, "label": "本編開始"}),
    )
    .unwrap();
    let ch = call_tool(
        &session,
        "manage_markers",
        &json!({"action": "youtube_chapters"}),
    )
    .unwrap();
    let ch_text = ch["content"][0]["text"].as_str().unwrap();
    assert!(ch_text.contains("00:12 本編開始"));

    call_tool(
        &session,
        "refine_subtitles",
        &json!({"mode": "local_wrap", "max_chars_per_line": 16}),
    )
    .unwrap();
    {
        let g = session.lock().unwrap();
        if let ItemKind::Text { content, .. } =
            &g.project.active_scene().unwrap().layers[1].items[0].kind
        {
            assert!(content.contains('\n'));
        }
    }

    assert!(call_tool(&session, "start_export", &json!({"user_confirmed": false})).is_err());
    let exp = call_tool(
        &session,
        "start_export",
        &json!({"user_confirmed": true, "format": "wav_audio_only", "file_name": "../escape_test"}),
    )
    .unwrap();
    let out_str = exp["content"][0]["text"].as_str().unwrap();
    assert!(out_str.contains("escape_test.wav"));
    assert!(!out_str.contains(".."));
}

#[test]
fn v06_mcp_tools_ducking_script_and_interop() {
    let proj = Project::new("McpV06", ProjectSettings::default());
    let session = Arc::new(Mutex::new(EditorSession::new(proj)));

    // 1. set_audio_ducking
    call_tool(
        &session,
        "set_audio_ducking",
        &json!({
            "enabled": true,
            "voice_layer_index": 3,
            "bgm_layer_index": 2,
            "reduction_db": -14.0
        }),
    )
    .unwrap();
    {
        let g = session.lock().unwrap();
        let d = g.project.active_scene().unwrap().audio_ducking;
        assert!(d.enabled);
        assert_eq!(d.voice_layer_index, 3);
        assert_eq!(d.bgm_layer_index, 2);
        assert!((d.reduction_db - (-14.0)).abs() < 1e-4);
    }

    // 2. import_script (Markdown dialogue)
    let md = "**霊夢** [smile]: こんにちは！\n**魔理沙**: v0.6 の新機能を紹介するぜ！\n";
    let res = call_tool(
        &session,
        "import_script",
        &json!({"content": md, "subtitle_layer": 1, "start_sec": 0.0}),
    )
    .unwrap();
    assert!(
        res["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("\"imported_lines\":2")
    );

    // 3. export_subtitles_or_otio (SRT + OTIO)
    let srt_res = call_tool(
        &session,
        "export_subtitles_or_otio",
        &json!({"format": "srt"}),
    )
    .unwrap();
    let srt_json: Value =
        serde_json::from_str(srt_res["content"][0]["text"].as_str().unwrap()).unwrap();
    let srt_body = srt_json["content"].as_str().unwrap();
    assert!(srt_body.contains("こんにちは！"));
    assert!(srt_body.contains("-->"));

    let otio_res = call_tool(
        &session,
        "export_subtitles_or_otio",
        &json!({"format": "otio"}),
    )
    .unwrap();
    assert!(
        otio_res["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Timeline.1")
    );

    // 4. import_subtitles_or_exo (SRT back into layer 0)
    let imp = call_tool(
        &session,
        "import_subtitles_or_exo",
        &json!({"format": "srt", "content": srt_body, "layer_index": 0}),
    )
    .unwrap();
    assert!(
        imp["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("\"imported_count\":2")
    );
}

#[test]
fn v07_mcp_tools_mask_wasm_and_extract_highlights() {
    let mut proj = Project::new("McpV07", ProjectSettings::default());
    let item = TimelineItem::new(
        "HeroSub",
        RationalTime::from_seconds(2.0),
        RationalTime::from_seconds(8.0),
        ItemKind::Text {
            content: "これはヤバい！すごい結論まとめポイント！".into(),
            font_family: "sans-serif".into(),
            font_size: 46.0,
            color: RgbaColor::WHITE,
            animator: TextAnimator::default(),
        },
    );
    let item_id = item.id;
    proj.active_scene_mut().unwrap().layers[1].items.push(item);
    let session = Arc::new(Mutex::new(EditorSession::new(proj)));

    // 1. set_bezier_mask
    call_tool(
        &session,
        "set_bezier_mask",
        &json!({"id": item_id.to_string(), "preset": "ellipse", "feather_px": 6.0}),
    )
    .unwrap();
    {
        let g = session.lock().unwrap();
        assert_eq!(
            g.project.active_scene().unwrap().layers[1].items[0]
                .masks
                .len(),
            1
        );
    }

    // 2. list_or_reload_wasm_plugins with emit_samples in temp dir
    let tmp_dir = std::env::temp_dir().join(format!("ve_mcp_v07_wasm_{}", Uuid::new_v4()));
    let wasm_res = call_tool(
        &session,
        "list_or_reload_wasm_plugins",
        &json!({"plugins_dir": tmp_dir.display().to_string(), "emit_samples": true}),
    )
    .unwrap();
    let _ = std::fs::remove_dir_all(&tmp_dir);
    let wasm_txt = wasm_res["content"][0]["text"].as_str().unwrap();
    assert!(wasm_txt.contains("wasm.wasm_invert"), "wasm_txt={wasm_txt}");

    // 3. extract_highlights + auto create 9:16 Shorts scene
    let hl_res = call_tool(
            &session,
            "extract_highlights",
            &json!({"clip_duration_sec": 5.0, "create_shorts_scene": true, "shorts_name": "AutoShorts"}),
        )
        .unwrap();
    let hl_txt = hl_res["content"][0]["text"].as_str().unwrap();
    assert!(hl_txt.contains("created_shorts_scene_id"));
    {
        let g = session.lock().unwrap();
        let s = g.project.active_scene().unwrap();
        assert_eq!(s.name, "AutoShorts");
        assert_eq!((s.width, s.height), (1080, 1920));
    }
}

#[test]
fn v071_mcp_tools_presets_and_marker_update() {
    let tmp_dir = std::env::temp_dir().join(format!("ve_mcp_v071_presets_{}", Uuid::new_v4()));
    let mut proj = Project::new("McpV071", ProjectSettings::default());
    let item = TimelineItem::new(
        "Sub",
        RationalTime::ZERO,
        RationalTime::from_seconds(3.0),
        ItemKind::Text {
            content: "プリセットテスト".into(),
            font_family: "sans-serif".into(),
            font_size: 40.0,
            color: RgbaColor::WHITE,
            animator: TextAnimator::default(),
        },
    );
    let item_id = item.id;
    proj.active_scene_mut().unwrap().layers[1].items.push(item);
    let folder = ve_core::ProjectFolder::init(&tmp_dir, &proj).unwrap();
    let mut ed = EditorSession::new(proj);
    ed.folder = Some(folder);
    let session = Arc::new(Mutex::new(ed));

    // 1. manage_markers add + update
    let added = call_tool(
        &session,
        "manage_markers",
        &json!({"action": "add", "time_sec": 4.0, "label": "仮ラベル"}),
    )
    .unwrap();
    let added_json: Value =
        serde_json::from_str(added["content"][0]["text"].as_str().unwrap()).unwrap();
    let mid = added_json["marker_id"].as_str().unwrap();

    call_tool(
            &session,
            "manage_markers",
            &json!({"action": "update", "marker_id": mid, "label": "確定チャプター", "color_hex": "#ff4466"}),
        )
        .unwrap();
    {
        let g = session.lock().unwrap();
        let m = &g.project.active_scene().unwrap().markers[0];
        assert_eq!(m.label, "確定チャプター");
        assert_eq!(m.color.to_hex_rgb(), "#ff4466");
    }

    // 2. manage_presets list + apply_text_style + save_text_style + apply_effect_preset + save_effect_preset
    let list = call_tool(&session, "manage_presets", &json!({"action": "list"})).unwrap();
    let list_txt = list["content"][0]["text"].as_str().unwrap();
    assert!(list_txt.contains("ゆっくり座布団字幕"));
    assert!(list_txt.contains("シネマティック暖色"));

    call_tool(
            &session,
            "manage_presets",
            &json!({"action": "apply_text_style", "id": item_id.to_string(), "name": "ゆっくり座布団字幕"}),
        )
        .unwrap();
    call_tool(
            &session,
            "manage_presets",
            &json!({"action": "apply_effect_preset", "id": item_id.to_string(), "name": "シネマティック暖色"}),
        )
        .unwrap();
    call_tool(
        &session,
        "manage_presets",
        &json!({"action": "save_text_style", "id": item_id.to_string(), "name": "カスタム座布団"}),
    )
    .unwrap();
    call_tool(
            &session,
            "manage_presets",
            &json!({"action": "save_effect_preset", "id": item_id.to_string(), "name": "カスタムシネマ"}),
        )
        .unwrap();

    let list2 = call_tool(&session, "manage_presets", &json!({"action": "list"})).unwrap();
    let list2_txt = list2["content"][0]["text"].as_str().unwrap();
    assert!(list2_txt.contains("カスタム座布団"));
    assert!(list2_txt.contains("カスタムシネマ"));
    {
        let g = session.lock().unwrap();
        let it = &g.project.active_scene().unwrap().layers[1].items[0];
        assert!(it.text_style.box_enabled);
        assert_eq!(it.effect_chain.len(), 2);
    }
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn v08_mcp_tools_text_edit_beats_and_color_grade() {
    let tmp_dir = std::env::temp_dir().join(format!("ve_mcp_v08_{}", Uuid::new_v4()));
    let mut proj = Project::new("McpV08", ProjectSettings::default());
    let mk_sub = |name: &str, s: f64, d: f64, txt: &str| {
        TimelineItem::new(
            name,
            RationalTime::from_seconds(s),
            RationalTime::from_seconds(d),
            ItemKind::Text {
                content: txt.into(),
                font_family: "sans-serif".into(),
                font_size: 44.0,
                color: RgbaColor::WHITE,
                animator: TextAnimator::default(),
            },
        )
    };
    proj.active_scene_mut().unwrap().layers[1].items.extend([
        mk_sub("T1", 0.0, 2.0, "えーと…"),
        mk_sub("T2", 2.0, 2.0, "本編スタート！"),
        mk_sub("T3", 4.0, 2.0, "まとめです"),
    ]);
    let folder = ve_core::ProjectFolder::init(&tmp_dir, &proj).unwrap();
    let mut ed = EditorSession::new(proj);
    ed.folder = Some(folder);
    let session = Arc::new(Mutex::new(ed));

    // 1. text_based_edit list + remove_fillers
    let listed = call_tool(&session, "text_based_edit", &json!({"action": "list"})).unwrap();
    assert!(
        listed["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("\"is_filler_only\":true")
    );
    call_tool(
        &session,
        "text_based_edit",
        &json!({"action": "remove_fillers"}),
    )
    .unwrap();
    {
        let g = session.lock().unwrap();
        let items = &g.project.active_scene().unwrap().layers[1].items;
        assert_eq!(items.len(), 2);
        assert!((items[0].start.as_seconds() - 0.0).abs() < 1e-6);
    }

    // 2. detect_beats (no audio -> safe zero result)
    let beats = call_tool(&session, "detect_beats", &json!({})).unwrap();
    assert!(
        beats["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("\"beat_count\":0")
    );

    // 3. set_color_grade + emit_sample_luts + builtin LUT
    let cg = call_tool(
        &session,
        "set_color_grade",
        &json!({
            "enabled": true,
            "exposure": 0.35,
            "temperature": 0.25,
            "gain": [1.1, 1.0, 0.9],
            "lut_path": "builtin:teal_orange",
            "lut_intensity": 0.75,
            "emit_sample_luts": true
        }),
    )
    .unwrap();
    let cg_txt = cg["content"][0]["text"].as_str().unwrap();
    assert!(cg_txt.contains("teal_orange_cinema.cube"));
    {
        let g = session.lock().unwrap();
        let grade = &g.project.active_scene().unwrap().color_grade;
        assert!(grade.enabled);
        assert_eq!(grade.lut_path.as_deref(), Some("builtin:teal_orange"));
        assert!((grade.exposure - 0.35).abs() < 1e-4);
    }
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn v09_mcp_tools_expressions_templates_and_fx() {
    let tmp_dir = std::env::temp_dir().join(format!("ve_mcp_v09_{}", Uuid::new_v4()));
    let proj = Project::new("McpV09", ProjectSettings::default());
    let folder = ve_core::ProjectFolder::init(&tmp_dir, &proj).unwrap();
    let mut ed = EditorSession::new(proj);
    ed.folder = Some(folder);
    let session = Arc::new(Mutex::new(ed));

    // 1. add_object particle_emitter & vector_shape
    let p_res = call_tool(
            &session,
            "add_object",
            &json!({"layer_index": 0, "name": "SnowFX", "kind": "particle_emitter", "preset": "snow", "duration_sec": 4.0}),
        )
        .unwrap();
    let p_id: String = serde_json::from_str::<Value>(p_res["content"][0]["text"].as_str().unwrap())
        .unwrap()["created_id"]
        .as_str()
        .unwrap()
        .to_string();

    let v_res = call_tool(
            &session,
            "add_object",
            &json!({"layer_index": 1, "name": "Wave", "kind": "vector_shape", "preset": "underline_wave", "duration_sec": 3.0}),
        )
        .unwrap();
    let v_id: String = serde_json::from_str::<Value>(v_res["content"][0]["text"].as_str().unwrap())
        .unwrap()["created_id"]
        .as_str()
        .unwrap()
        .to_string();

    // 2. set_expression set + evaluate + bake
    call_tool(
            &session,
            "set_expression",
            &json!({"action": "set", "id": v_id, "property": "trim_end", "expression": "ease(local_time, 0, 1, 0, 1)"}),
        )
        .unwrap();
    let eval = call_tool(
        &session,
        "set_expression",
        &json!({"action": "evaluate", "id": v_id, "property": "trim_end", "time_sec": 0.5}),
    )
    .unwrap();
    let eval_v = serde_json::from_str::<Value>(eval["content"][0]["text"].as_str().unwrap())
        .unwrap()["value"]
        .as_f64()
        .unwrap();
    assert!((eval_v - 0.5).abs() < 1e-2);
    call_tool(
        &session,
        "set_expression",
        &json!({"action": "bake", "id": v_id, "property": "trim_end", "step_frames": 15}),
    )
    .unwrap();

    // 3. manage_templates list + instantiate + update_instance
    let t_list = call_tool(&session, "manage_templates", &json!({"action": "list"})).unwrap();
    assert!(
        t_list["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("builtin_lower_third_news")
    );
    let inst = call_tool(
        &session,
        "manage_templates",
        &json!({
            "action": "instantiate",
            "template_id": "builtin_lower_third_news",
            "layer_index": 2,
            "start_sec": 0.0,
            "duration_sec": 6.0,
            "values": { "title": "AI テンプレート速報" }
        }),
    )
    .unwrap();
    let inst_id = serde_json::from_str::<Value>(inst["content"][0]["text"].as_str().unwrap())
        .unwrap()["created_item_id"]
        .as_str()
        .unwrap()
        .to_string();
    call_tool(
        &session,
        "manage_templates",
        &json!({
            "action": "update_instance",
            "item_id": inst_id,
            "duration_sec": 8.0,
            "values": { "title": "更新後見出し", "accent_color": "#ff4488" }
        }),
    )
    .unwrap();

    // 4. set_scene_motion_blur + set_light_3d + set_particle
    call_tool(
            &session,
            "set_scene_motion_blur",
            &json!({"action": "set_motion_blur", "enabled": true, "shutter_angle_deg": 240.0, "preview_samples": 4}),
        )
        .unwrap();
    call_tool(
            &session,
            "set_scene_motion_blur",
            &json!({"action": "set_light_3d", "enabled": true, "light_type": "point", "intensity": 1.6, "cast_shadow": true}),
        )
        .unwrap();
    call_tool(
        &session,
        "set_scene_motion_blur",
        &json!({"action": "set_particle", "item_id": p_id, "count": 180}),
    )
    .unwrap();

    {
        let g = session.lock().unwrap();
        let s = g.project.active_scene().unwrap();
        assert!(s.motion_blur.enabled);
        assert!((s.motion_blur.shutter_angle_deg - 240.0).abs() < 1e-3);
        assert!(s.camera.light.enabled);
        assert_eq!(s.camera.light.light_type, Light3DType::Point);
    }

    // Verify exact MCP tool count = 58 (v2.0)
    let defs = tool_definitions();
    assert_eq!(defs.as_array().unwrap().len(), 82);

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn v10_mcp_tools_multicam_nle_delivery_and_ecosystem() {
    let tmp_dir = std::env::temp_dir().join(format!("ve_mcp_v10_{}", Uuid::new_v4()));
    let proj = Project::new("McpV10", ProjectSettings::default());
    let folder = ve_core::ProjectFolder::init(&tmp_dir, &proj).unwrap();
    let mut ed = EditorSession::new(proj);
    ed.folder = Some(folder);
    let session = Arc::new(Mutex::new(ed));

    // 1. Add two camera clips and test sync_multicam (manual_offsets + apply_switch_cuts)
    let c1_res = call_tool(
            &session,
            "add_object",
            &json!({"layer_index": 0, "name": "Cam1", "kind": "shape", "start_sec": 0.0, "duration_sec": 6.0}),
        )
        .unwrap();
    let c1_id = serde_json::from_str::<Value>(c1_res["content"][0]["text"].as_str().unwrap())
        .unwrap()["created_id"]
        .as_str()
        .unwrap()
        .to_string();
    let c2_res = call_tool(
            &session,
            "add_object",
            &json!({"layer_index": 1, "name": "Cam2", "kind": "shape", "start_sec": 0.5, "duration_sec": 6.0}),
        )
        .unwrap();
    let c2_id = serde_json::from_str::<Value>(c2_res["content"][0]["text"].as_str().unwrap())
        .unwrap()["created_id"]
        .as_str()
        .unwrap()
        .to_string();

    call_tool(
        &session,
        "sync_multicam",
        &json!({
            "action": "auto_sync_audio",
            "manual_offsets": [
                {"id": c1_id, "offset_sec": 0.0},
                {"id": c2_id, "offset_sec": -0.5}
            ],
            "apply": true
        }),
    )
    .unwrap();
    let sw_res = call_tool(
        &session,
        "sync_multicam",
        &json!({
            "action": "apply_switch_cuts",
            "angle_layers": [0, 1],
            "output_layer_index": 2,
            "range_start_sec": 0.0,
            "range_end_sec": 5.0,
            "cuts": [
                {"time_sec": 0.0, "active_angle": 0},
                {"time_sec": 2.5, "active_angle": 1}
            ]
        }),
    )
    .unwrap();
    assert!(
        sw_res["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("\"generated_clips\":2")
    );

    // 2. nle_pro_edit: group + unlink + set_layer_mixer
    call_tool(
        &session,
        "nle_pro_edit",
        &json!({"action": "group", "ids": [c1_id, c2_id]}),
    )
    .unwrap();
    call_tool(
        &session,
        "nle_pro_edit",
        &json!({
            "action": "set_layer_mixer",
            "layer_index": 3,
            "volume": 0.8,
            "pan": -0.5,
            "solo": true,
            "bus": "Voice",
            "send_reverb": 0.3
        }),
    )
    .unwrap();
    {
        let g = session.lock().unwrap();
        let s = g.project.active_scene().unwrap();
        assert!(s.layers[0].items[0].group_id.is_some());
        assert_eq!(s.layers[0].items[0].group_id, s.layers[1].items[0].group_id);
        assert!(s.layers[3].solo);
        assert!((s.layers[3].pan - (-0.5)).abs() < 1e-4);
        assert_eq!(s.layers[3].bus, "Voice");
    }

    // 3. validate_and_package_project: validate + archive_zip
    let val = call_tool(
        &session,
        "validate_and_package_project",
        &json!({"action": "validate"}),
    )
    .unwrap();
    assert!(
        val["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("\"passed\":true")
    );
    let zip_res = call_tool(
        &session,
        "validate_and_package_project",
        &json!({"action": "archive_zip", "file_name": "v10_release_pack"}),
    )
    .unwrap();
    let zip_json: Value =
        serde_json::from_str(zip_res["content"][0]["text"].as_str().unwrap()).unwrap();
    assert!(zip_json["file_count"].as_u64().unwrap() >= 1);
    assert!(std::path::Path::new(zip_json["zip_path"].as_str().unwrap()).exists());

    // 4. manage_ecosystem: settings + keymap json + SHA-256 registry install + hw_status
    call_tool(
        &session,
        "manage_ecosystem",
        &json!({
            "action": "set_settings",
            "language": "en",
            "theme": "high_contrast",
            "font_scale": 1.15,
            "keymap_preset": "premiere",
            "parallel_export_threads": 4
        }),
    )
    .unwrap();
    let km = call_tool(
        &session,
        "manage_ecosystem",
        &json!({"action": "export_keymap_json"}),
    )
    .unwrap();
    let km_txt = km["content"][0]["text"].as_str().unwrap();
    assert!(km_txt.contains("premiere"));
    call_tool(
        &session,
        "manage_ecosystem",
        &json!({"action": "import_keymap_json", "keymap_json": km_txt}),
    )
    .unwrap();
    let reg = call_tool(
        &session,
        "manage_ecosystem",
        &json!({"action": "list_registry"}),
    )
    .unwrap();
    assert!(
        reg["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("official_wgsl_film_vignette")
    );
    let inst = call_tool(
        &session,
        "manage_ecosystem",
        &json!({"action": "install_extension", "extension_id": "official_wgsl_film_vignette"}),
    )
    .unwrap();
    assert!(
        inst["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("sha256_verified")
    );
    let hw = call_tool(
        &session,
        "manage_ecosystem",
        &json!({"action": "hw_status"}),
    )
    .unwrap();
    assert!(
        hw["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("resolved_hw_decode")
    );

    let defs = tool_definitions();
    assert_eq!(defs.as_array().unwrap().len(), 82);
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn v20_mcp_tools_node_graph_ai_vision_director_and_collab() {
    let tmp_dir = std::env::temp_dir().join(format!("ve_mcp_v20_{}", Uuid::new_v4()));
    let proj = Project::new("McpV20", ProjectSettings::default());
    let folder = ve_core::ProjectFolder::init(&tmp_dir, &proj).unwrap();
    let mut ed = EditorSession::new(proj);
    ed.folder = Some(folder);
    let session = Arc::new(Mutex::new(ed));

    // 0. Add a subject shape clip
    let obj_res = call_tool(
            &session,
            "add_object",
            &json!({"layer_index": 0, "name": "Subject", "kind": "shape", "start_sec": 0.0, "duration_sec": 4.0, "width": 320.0, "height": 240.0}),
        )
        .unwrap();
    let obj_id = serde_json::from_str::<Value>(obj_res["content"][0]["text"].as_str().unwrap())
        .unwrap()["created_id"]
        .as_str()
        .unwrap()
        .to_string();

    // 1. #21 manage_node_graph: from_layers -> add_node -> validate -> sync_to_layers
    let ng_gen = call_tool(
        &session,
        "manage_node_graph",
        &json!({"action": "from_layers"}),
    )
    .unwrap();
    assert!(
        ng_gen["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("generated_from_layers")
    );
    let ng_add = call_tool(
            &session,
            "manage_node_graph",
            &json!({"action": "add_node", "node_type": "channel_split", "name": "LumaSplit", "channel": "luma"}),
        )
        .unwrap();
    assert!(
        ng_add["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("created_node_id")
    );
    let ng_val = call_tool(
        &session,
        "manage_node_graph",
        &json!({"action": "validate"}),
    )
    .unwrap();
    assert!(
        ng_val["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("\"valid\":true")
    );
    call_tool(
        &session,
        "manage_node_graph",
        &json!({"action": "sync_to_layers"}),
    )
    .unwrap();

    // 2. #22 run_local_ai_vision: status + segmentation + super_res + optical_flow + auto_reframe
    let ai_st = call_tool(
        &session,
        "run_local_ai_vision",
        &json!({"action": "status"}),
    )
    .unwrap();
    assert!(
        ai_st["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("\"local_only_guaranteed\": true")
    );
    call_tool(
        &session,
        "run_local_ai_vision",
        &json!({"action": "apply_segmentation", "id": obj_id, "sensitivity": 0.6}),
    )
    .unwrap();
    call_tool(
        &session,
        "run_local_ai_vision",
        &json!({"action": "apply_super_resolution", "id": obj_id, "sharpen": 0.5}),
    )
    .unwrap();
    call_tool(
        &session,
        "run_local_ai_vision",
        &json!({"action": "set_optical_flow_slowmo", "id": obj_id, "enabled": true, "speed": 0.5}),
    )
    .unwrap();
    let rf = call_tool(
            &session,
            "run_local_ai_vision",
            &json!({"action": "auto_reframe", "target_width": 1080, "target_height": 1920, "apply": true}),
        )
        .unwrap();
    assert!(
        rf["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("\"applied\": true")
    );
    {
        let g = session.lock().unwrap();
        let s = g.project.active_scene().unwrap();
        assert_eq!((s.width, s.height), (1080, 1920));
        assert!(s.layers[0].items[0].playback.optical_flow_interp);
        assert!(
            s.layers[0].items[0]
                .effect_chain
                .iter()
                .any(|e| e.effect_type == "ai_segment")
        );
    }

    // 3. #23 orchestrate_edit_recipe: list -> start_run (pauses at human approval gate) -> approve_gate -> completed -> export_yaml -> record_from_history
    let rec_list = call_tool(
        &session,
        "orchestrate_edit_recipe",
        &json!({"action": "list_recipes"}),
    )
    .unwrap();
    assert!(
        rec_list["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("YouTube")
    );

    let run_res = call_tool(
        &session,
        "orchestrate_edit_recipe",
        &json!({
            "action": "start_run",
            "recipe_name": "YouTube 解説動画・標準ディレクターパイプライン",
            "variables": { "title": "v2.0 MCP テストタイトル" },
            "auto_run": true
        }),
    )
    .unwrap();
    let run_json: Value =
        serde_json::from_str(run_res["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(run_json["status"], "awaiting_approval");

    let approved_res = call_tool(
        &session,
        "orchestrate_edit_recipe",
        &json!({"action": "approve_gate", "continue_after": true}),
    )
    .unwrap();
    let approved_json: Value =
        serde_json::from_str(approved_res["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(approved_json["status"], "completed");
    assert_eq!(approved_json["logs"].as_array().unwrap().len(), 5);

    let yaml_res = call_tool(
        &session,
        "orchestrate_edit_recipe",
        &json!({"action": "export_yaml"}),
    )
    .unwrap();
    assert!(
        yaml_res["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("steps:")
    );

    let rec_hist = call_tool(
        &session,
        "orchestrate_edit_recipe",
        &json!({"action": "record_from_history", "name": "AutoRecorded"}),
    )
    .unwrap();
    assert!(
        rec_hist["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("\"status\": \"recorded\"")
    );

    // 4. #24 collab_diff_and_review: snapshot -> branch edit -> diff -> 3-way merge -> Frame.io/YouTube review import -> markers
    call_tool(
        &session,
        "collab_diff_and_review",
        &json!({"action": "save_snapshot", "name": "base_main"}),
    )
    .unwrap();
    call_tool(
            &session,
            "add_object",
            &json!({"layer_index": 2, "name": "BranchB_Title", "kind": "text", "content": "Branch B", "start_sec": 1.0, "duration_sec": 2.0}),
        )
        .unwrap();
    call_tool(
        &session,
        "collab_diff_and_review",
        &json!({"action": "save_snapshot", "name": "branch_b"}),
    )
    .unwrap();
    // Rollback local main 1 step (Undo BranchB_Title) and add BranchA_Marker instead
    {
        let mut g = session.lock().unwrap();
        let s = &mut *g;
        assert!(s.bus.undo(&mut s.project));
    }
    call_tool(
        &session,
        "manage_markers",
        &json!({"action": "add", "time_sec": 2.0, "label": "BranchA Marker"}),
    )
    .unwrap();
    let diff_res = call_tool(
            &session,
            "collab_diff_and_review",
            &json!({"action": "diff_snapshots", "base_snapshot": "base_main", "target_snapshot": "branch_b"}),
        )
        .unwrap();
    assert!(
        diff_res["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("BranchB_Title")
    );

    let merge_res = call_tool(
            &session,
            "collab_diff_and_review",
            &json!({"action": "merge_three_way", "base_snapshot": "base_main", "theirs_snapshot": "branch_b", "apply": true}),
        )
        .unwrap();
    let merge_json: Value =
        serde_json::from_str(merge_res["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(merge_json["conflict_count"], 0);
    assert_eq!(merge_json["applied"], true);

    let rev_imp = call_tool(
            &session,
            "collab_diff_and_review",
            &json!({
                "action": "import_reviews",
                "format": "frameio_csv",
                "content": "Timecode In,Commenter,Comment,Status\n00:00:01:00,Director,テロップもう少し太く,Open\n"
            }),
        )
        .unwrap();
    assert!(
        rev_imp["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("\"imported_count\":1")
    );
    let mk_res = call_tool(
        &session,
        "collab_diff_and_review",
        &json!({"action": "convert_reviews_to_markers", "only_unresolved": true}),
    )
    .unwrap();
    assert!(
        mk_res["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("\"converted_markers\":1")
    );

    let defs = tool_definitions();
    assert_eq!(defs.as_array().unwrap().len(), 82);
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn v21_mcp_tools_color_vfx_spatial_and_kinetic() {
    let proj = Project::new("McpV21", ProjectSettings::default());
    let session = Arc::new(Mutex::new(EditorSession::new(proj)));

    // 0. Add a shape clip + 2 text clips
    let shp_res = call_tool(
            &session,
            "add_object",
            &json!({"layer_index": 0, "name": "MonitorScreen", "kind": "shape", "start_sec": 0.0, "duration_sec": 3.0, "width": 320.0, "height": 240.0}),
        )
        .unwrap();
    let shp_id = serde_json::from_str::<Value>(shp_res["content"][0]["text"].as_str().unwrap())
        .unwrap()["created_id"]
        .as_str()
        .unwrap()
        .to_string();

    let sub1_res = call_tool(
            &session,
            "add_object",
            &json!({"layer_index": 1, "name": "Sub1", "kind": "text", "content": "重要な ポイント です", "start_sec": 0.0, "duration_sec": 2.0}),
        )
        .unwrap();
    let sub1_id = serde_json::from_str::<Value>(sub1_res["content"][0]["text"].as_str().unwrap())
        .unwrap()["created_id"]
        .as_str()
        .unwrap()
        .to_string();

    let sub2_res = call_tool(
            &session,
            "add_object",
            &json!({"layer_index": 1, "name": "Sub2", "kind": "text", "content": "今すぐ チェック！", "start_sec": 2.0, "duration_sec": 2.0}),
        )
        .unwrap();
    let sub2_id = serde_json::from_str::<Value>(sub2_res["content"][0]["text"].as_str().unwrap())
        .unwrap()["created_id"]
        .as_str()
        .unwrap()
        .to_string();

    // 1. #25 pro_color_match_and_grade
    call_tool(
        &session,
        "pro_color_match_and_grade",
        &json!({"action": "set_curves", "preset": "s_curve"}),
    )
    .unwrap();
    call_tool(
            &session,
            "pro_color_match_and_grade",
            &json!({"action": "set_hsl_qualifier", "hue_center_deg": 210.0, "sat_scale": 1.35, "show_matte_overlay": false}),
        )
        .unwrap();
    let cm = call_tool(
            &session,
            "pro_color_match_and_grade",
            &json!({"action": "auto_match_timecodes", "reference_time_sec": 0.2, "target_time_sec": 1.5, "apply": true}),
        )
        .unwrap();
    assert!(
        cm["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("\"applied\":true")
    );
    {
        let g = session.lock().unwrap();
        let cg = &g.project.active_scene().unwrap().color_grade;
        assert!(cg.enabled);
        assert!(cg.curves.enabled);
        assert!(cg.hsl_qualifier.enabled);
        assert!((cg.hsl_qualifier.hue_center_deg - 210.0).abs() < 1e-3);
    }

    // 2. #26 vfx_stabilize_and_corner_pin
    call_tool(
            &session,
            "vfx_stabilize_and_corner_pin",
            &json!({"action": "set_corner_pin", "id": shp_id, "preset": "monitor_tilt", "feather_px": 2.0}),
        )
        .unwrap();
    let stab = call_tool(
        &session,
        "vfx_stabilize_and_corner_pin",
        &json!({"action": "analyze_and_stabilize", "id": shp_id, "smoothness": 0.8, "apply": true}),
    )
    .unwrap();
    assert!(
        stab["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("\"applied\":true")
    );
    {
        let g = session.lock().unwrap();
        let shp = &g.project.active_scene().unwrap().layers[0].items[0];
        assert!(shp.corner_pin.enabled);
        assert!(!shp.keyframes.is_empty());
    }

    // 3. #27 spatial_and_master_audio
    call_tool(
            &session,
            "spatial_and_master_audio",
            &json!({"action": "add_spatial_3d", "azimuth_deg": 65.0, "distance": 1.2, "room_mix": 0.15}),
        )
        .unwrap();
    call_tool(
        &session,
        "spatial_and_master_audio",
        &json!({"action": "add_multiband_comp", "ratio": 3.5, "makeup_db": 1.0}),
    )
    .unwrap();
    call_tool(
        &session,
        "spatial_and_master_audio",
        &json!({"action": "add_true_peak_limiter", "ceiling_dbtp": -1.0, "release_ms": 50.0}),
    )
    .unwrap();
    let tp = call_tool(
        &session,
        "spatial_and_master_audio",
        &json!({"action": "measure_master_true_peak", "duration_sec": 1.0}),
    )
    .unwrap();
    assert!(
        tp["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("\"broadcast_safe\":true")
    );
    {
        let g = session.lock().unwrap();
        let mfx = &g.project.active_scene().unwrap().master_audio_effects;
        assert_eq!(mfx.len(), 3);
        assert_eq!(mfx[0].effect_type, "spatial_3d");
        assert_eq!(mfx[1].effect_type, "multiband_comp");
        assert_eq!(mfx[2].effect_type, "true_peak_limiter");
    }

    // 4. #28 kinetic_subtitles_and_localize
    let kin = call_tool(
        &session,
        "kinetic_subtitles_and_localize",
        &json!({
            "action": "set_kinetic_style",
            "mode": "karaoke_sweep",
            "pop_scale": 1.25,
            "active_color": "#ffdd33"
        }),
    )
    .unwrap();
    assert!(
        kin["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("\"updated_count\":2")
    );

    let csv_res = call_tool(
            &session,
            "kinetic_subtitles_and_localize",
            &json!({"action": "extract_localization_table", "source_locale": "ja", "target_locales": ["en"], "format": "csv"}),
        )
        .unwrap();
    let csv_txt = csv_res["content"][0]["text"].as_str().unwrap();
    assert!(csv_txt.contains("item_id,start_sec,item_name,ja,en"));

    let loc_apply = call_tool(
        &session,
        "kinetic_subtitles_and_localize",
        &json!({
            "action": "apply_locale_translations",
            "locale": "en",
            "translations": {
                sub1_id.clone(): "Important key point",
                sub2_id.clone(): "Check it out now!"
            }
        }),
    )
    .unwrap();
    assert!(
        loc_apply["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("\"applied_count\":2")
    );
    {
        let g = session.lock().unwrap();
        let subs = &g.project.active_scene().unwrap().layers[1].items;
        assert!(subs[0].kinetic_subtitle.enabled);
        if let ItemKind::Text { content, .. } = &subs[0].kind {
            assert_eq!(content, "Important key point");
        } else {
            panic!("expected text item");
        }
    }
    // 1-Undo reverts both localized subtitles back to Japanese
    {
        let mut g = session.lock().unwrap();
        let s = &mut *g;
        assert!(s.bus.undo(&mut s.project));
        if let ItemKind::Text { content, .. } =
            &s.project.active_scene().unwrap().layers[1].items[0].kind
        {
            assert_eq!(content, "重要な ポイント です");
        }
    }

    let defs = tool_definitions();
    assert_eq!(defs.as_array().unwrap().len(), 82);
}

#[test]
fn v30_mcp_tools_gpu_3d_dubbing_and_crdt() {
    let tmp_dir = std::env::temp_dir().join(format!("ve_mcp_v30_{}", Uuid::new_v4()));
    let proj = Project::new("McpV30", ProjectSettings::default());
    let folder = ve_core::ProjectFolder::init(&tmp_dir, &proj).unwrap();
    let mut ed = EditorSession::with_folder(proj, folder);
    ed.project.active_scene_mut().unwrap().layers[1]
        .items
        .push(TimelineItem::new(
            "SubJP",
            RationalTime::ZERO,
            RationalTime::from_seconds(2.0),
            ItemKind::Text {
                content: "こんにちは、次世代編集ソフトです！".into(),
                font_family: "Sans".into(),
                font_size: 42.0,
                color: RgbaColor::WHITE,
                animator: TextAnimator::default(),
            },
        ));
    let sub_id = ed.project.active_scene().unwrap().layers[1].items[0]
        .id
        .to_string();
    let session = Arc::new(Mutex::new(ed));

    // 1. #29 gpu_v30_pipeline: status + benchmark_4k + run_grade_and_scopes
    let gpu_st = call_tool(&session, "gpu_v30_pipeline", &json!({"action": "status"})).unwrap();
    let st_txt = gpu_st["content"][0]["text"].as_str().unwrap();
    assert!(st_txt.contains("\"color_shader_valid\":true"));
    assert!(st_txt.contains("\"scopes_shader_valid\":true"));

    let bench = call_tool(
        &session,
        "gpu_v30_pipeline",
        &json!({"action": "benchmark_4k", "width": 320, "height": 180}),
    )
    .unwrap();
    assert!(
        bench["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("\"estimated_fps\"")
    );

    // 2. #30 scene_3d_and_camera_track: add_mesh_3d + update_mesh_3d + emit_sample_glb + track_3d_camera
    let m3d = call_tool(
        &session,
        "scene_3d_and_camera_track",
        &json!({
            "action": "add_mesh_3d",
            "layer_index": 0,
            "name": "Hero3D",
            "preset": "extruded_text",
            "text_content": "V3.0",
            "metallic": 0.7,
            "roughness": 0.2
        }),
    )
    .unwrap();
    let m3d_id = serde_json::from_str::<Value>(m3d["content"][0]["text"].as_str().unwrap())
        .unwrap()["created_id"]
        .as_str()
        .unwrap()
        .to_string();

    call_tool(
        &session,
        "scene_3d_and_camera_track",
        &json!({"action": "update_mesh_3d", "id": m3d_id, "preset": "torus", "wireframe": true}),
    )
    .unwrap();

    let glb = call_tool(
        &session,
        "scene_3d_and_camera_track",
        &json!({"action": "emit_sample_glb"}),
    )
    .unwrap();
    assert!(
        glb["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("\"triangles\":12")
    );

    let cam_tr = call_tool(
        &session,
        "scene_3d_and_camera_track",
        &json!({"action": "track_3d_camera", "sample_interval_sec": 0.5, "apply": true}),
    )
    .unwrap();
    assert!(
        cam_tr["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("\"applied\":true")
    );

    // 3. #31 offline_ai_dubbing_lipsync: list_voices + align_phonemes + generate_locale_dubbing
    let voices = call_tool(
        &session,
        "offline_ai_dubbing_lipsync",
        &json!({"action": "list_voices"}),
    )
    .unwrap();
    assert!(
        voices["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("\"offline_only\":true")
    );

    let ph = call_tool(
        &session,
        "offline_ai_dubbing_lipsync",
        &json!({"action": "align_phonemes", "text": "Hello V3 world!", "duration_sec": 2.0}),
    )
    .unwrap();
    assert!(
        ph["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("\"phoneme_count\"")
    );

    let dub = call_tool(
        &session,
        "offline_ai_dubbing_lipsync",
        &json!({
            "action": "generate_locale_dubbing",
            "locale": "en",
            "voice": "en_female",
            "audio_layer_index": 3,
            "translations": { sub_id.clone(): "Hello, welcome to version 3!" },
            "apply": true
        }),
    )
    .unwrap();
    let dub_json: Value =
        serde_json::from_str(dub["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(dub_json["generated_clips"], 1);
    assert_eq!(dub_json["applied"], true);
    let wav_p = dub_json["wav_paths"][0].as_str().unwrap();
    assert!(std::path::Path::new(wav_p).exists());

    // 4. #32 crdt_realtime_collab: profile + cursor + marker op + export_delta + remote merge
    call_tool(
            &session,
            "crdt_realtime_collab",
            &json!({"action": "set_peer_profile", "peer_id": "editor_a", "display_name": "Alice", "color_hex": "#33ddff"}),
        )
        .unwrap();
    call_tool(
        &session,
        "crdt_realtime_collab",
        &json!({"action": "update_cursor", "playhead_sec": 1.5}),
    )
    .unwrap();
    call_tool(
        &session,
        "crdt_realtime_collab",
        &json!({"action": "record_marker_op", "time_sec": 0.8, "label": "Alice Note"}),
    )
    .unwrap();

    let mut remote_peer = CrdtSyncLog::new("editor_b", "Bob", "#ff6699");
    remote_peer.update_local_cursor(2.2, None);
    remote_peer.record_local(CrdtOpKind::AddMarker {
        marker: TimelineMarker::new(RationalTime::from_seconds(1.8), "Bob Remote Marker"),
    });
    let remote_delta = remote_peer.export_delta_json(None);

    let merged = call_tool(
        &session,
        "crdt_realtime_collab",
        &json!({"action": "merge_remote_delta", "delta_json": remote_delta, "apply": true}),
    )
    .unwrap();
    let merged_json: Value =
        serde_json::from_str(merged["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(merged_json["merged_new_ops"], 2);
    assert_eq!(merged_json["applied"], true);
    {
        let g = session.lock().unwrap();
        let markers = &g.project.active_scene().unwrap().markers;
        assert!(markers.iter().any(|m| m.label == "Bob Remote Marker"));
        assert!(g.crdt_log.peers.contains_key("editor_b"));
    }

    let defs = tool_definitions();
    assert_eq!(defs.as_array().unwrap().len(), 82);
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn v40_mcp_tools_aces_physics_generative_and_qc() {
    let tmp_dir = std::env::temp_dir().join(format!("ve_mcp_v40_{}", Uuid::new_v4()));
    std::fs::create_dir_all(&tmp_dir).unwrap();
    let mut proj = Project::new("v4.0 Master Studio Test", ProjectSettings::default());
    proj.active_scene_mut().unwrap().width = 320;
    proj.active_scene_mut().unwrap().height = 180;
    let folder = ve_core::ProjectFolder::init(&tmp_dir, &proj).unwrap();
    let session = std::sync::Arc::new(std::sync::Mutex::new(EditorSession::with_folder(
        proj, folder,
    )));

    // 1. #33 aces_color_management: set_config + export_ocio
    let ocio_path = tmp_dir.join("studio_v4.ocio");
    let cm_res = call_tool(
        &session,
        "aces_color_management",
        &json!({
            "action": "set_config",
            "enabled": true,
            "input_idt": "s_log3",
            "working_space": "aces_cg",
            "output_odt": "display_p3",
            "tone_mapper": "ag_x",
            "exposure_stops": 0.4,
            "gamut_compress": true
        }),
    )
    .unwrap();
    let cm_json: Value =
        serde_json::from_str(cm_res["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(cm_json["applied"], true);
    assert_eq!(cm_json["config"]["tone_mapper"], "ag_x");

    let ocio_res = call_tool(
        &session,
        "aces_color_management",
        &json!({
            "action": "export_ocio",
            "output_path": ocio_path.display().to_string()
        }),
    )
    .unwrap();
    let ocio_json: Value =
        serde_json::from_str(ocio_res["content"][0]["text"].as_str().unwrap()).unwrap();
    assert!(
        ocio_json["ocio_yaml"]
            .as_str()
            .unwrap()
            .contains("ocio_profile_version: 2")
    );
    assert!(ocio_path.exists());

    // 2. #35 generative_bg_and_foley: add_generative_bg + update + synthesize_foley_sfx
    let bg_res = call_tool(
        &session,
        "generative_bg_and_foley",
        &json!({
            "action": "add_generative_bg",
            "preset": "cyber_grid3d",
            "layer_index": 0,
            "start_sec": 0.0,
            "duration_sec": 3.0,
            "speed": 1.4
        }),
    )
    .unwrap();
    let bg_json: Value =
        serde_json::from_str(bg_res["content"][0]["text"].as_str().unwrap()).unwrap();
    let bg_id = bg_json["item_id"].as_str().unwrap().to_string();

    call_tool(
        &session,
        "generative_bg_and_foley",
        &json!({
            "action": "update_generative_bg",
            "id": bg_id,
            "preset": "aurora_borealis",
            "complexity": 5
        }),
    )
    .unwrap();

    let sfx_res = call_tool(
        &session,
        "generative_bg_and_foley",
        &json!({
            "action": "synthesize_foley_sfx",
            "preset": "sub_bass_impact",
            "duration_sec": 0.8,
            "audio_layer_index": 3,
            "apply": true
        }),
    )
    .unwrap();
    let sfx_json: Value =
        serde_json::from_str(sfx_res["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(sfx_json["applied"], true);
    assert!(std::path::Path::new(sfx_json["wav_path"].as_str().unwrap()).exists());

    // 3. #34 physics_simulate_and_bake: add a title text item, preview trajectory & bake keyframes
    let txt = call_tool(
        &session,
        "add_object",
        &json!({
            "layer_index": 1,
            "name": "PhysicsTitle",
            "kind": "text",
            "content": "PHYSICS V4",
            "start_sec": 0.0,
            "duration_sec": 3.0
        }),
    )
    .unwrap();
    let txt_id = serde_json::from_str::<Value>(txt["content"][0]["text"].as_str().unwrap())
        .unwrap()["created_id"]
        .as_str()
        .unwrap()
        .to_string();

    let phys_prev = call_tool(
        &session,
        "physics_simulate_and_bake",
        &json!({
            "action": "preview_trajectory",
            "item_ids": [txt_id.clone()],
            "preset": "bouncing_title",
            "duration_sec": 2.0
        }),
    )
    .unwrap();
    let phys_prev_json: Value =
        serde_json::from_str(phys_prev["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(phys_prev_json["body_count"], 1);
    assert!(phys_prev_json["total_bounces"].as_u64().unwrap() >= 1);

    let phys_bake = call_tool(
        &session,
        "physics_simulate_and_bake",
        &json!({
            "action": "bake_keyframes",
            "item_ids": [txt_id],
            "preset": "bouncing_title",
            "duration_sec": 2.0
        }),
    )
    .unwrap();
    let phys_bake_json: Value =
        serde_json::from_str(phys_bake["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(phys_bake_json["baked"], true);
    assert!(phys_bake_json["keyframe_commands"].as_u64().unwrap() >= 30);

    // 4. #36 broadcast_qc_and_web_export: run_qc_audit + generate_hero_thumbnail + export_web_review_html
    let qc_res = call_tool(
        &session,
        "broadcast_qc_and_web_export",
        &json!({
            "action": "run_qc_audit",
            "profile": "youtube1080p",
            "sample_count": 4
        }),
    )
    .unwrap();
    let qc_json: Value =
        serde_json::from_str(qc_res["content"][0]["text"].as_str().unwrap()).unwrap();
    assert!(qc_json["checks"].as_array().unwrap().len() >= 6);

    let thumb_path = tmp_dir.join("hero_thumb.png");
    let thumb_res = call_tool(
        &session,
        "broadcast_qc_and_web_export",
        &json!({
            "action": "generate_hero_thumbnail",
            "sample_count": 4,
            "output_path": thumb_path.display().to_string()
        }),
    )
    .unwrap();
    let thumb_json: Value =
        serde_json::from_str(thumb_res["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(thumb_json["width"], 1280);
    assert_eq!(thumb_json["height"], 720);
    assert!(thumb_path.exists());

    let html_path = tmp_dir.join("review_player.html");
    let html_res = call_tool(
        &session,
        "broadcast_qc_and_web_export",
        &json!({
            "action": "export_web_review_html",
            "profile": "youtube1080p",
            "sample_count": 4,
            "output_path": html_path.display().to_string()
        }),
    )
    .unwrap();
    let html_json: Value =
        serde_json::from_str(html_res["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(html_json["embedded_frames"], 4);
    assert!(html_path.exists());
    let html_body = std::fs::read_to_string(&html_path).unwrap();
    assert!(html_body.contains("data:image/png;base64,"));

    let defs = tool_definitions();
    assert_eq!(defs.as_array().unwrap().len(), 82);
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn v50_mcp_tools_rig2d_bgm_speed_ramp_and_finishing() {
    let tmp_dir = std::env::temp_dir().join(format!("ve_mcp_v50_{}", Uuid::new_v4()));
    std::fs::create_dir_all(&tmp_dir).unwrap();
    let mut proj = Project::new("v5.0 Pro Finishing Test", ProjectSettings::default());
    proj.active_scene_mut().unwrap().width = 160;
    proj.active_scene_mut().unwrap().height = 90;
    let folder = ve_core::ProjectFolder::init(&tmp_dir, &proj).unwrap();
    let session = std::sync::Arc::new(std::sync::Mutex::new(EditorSession::with_folder(
        proj, folder,
    )));

    // 1. #37 rig2d_puppet_control: add + update + solve_ik_preview
    let rig_add = call_tool(
        &session,
        "rig2d_puppet_control",
        &json!({
            "action": "add_rigged_puppet",
            "preset": "wave_hello",
            "layer_index": 1,
            "duration_sec": 3.0,
            "right_ik_enabled": true,
            "right_ik_x": 80.0,
            "right_ik_y": -85.0
        }),
    )
    .unwrap();
    let rig_json: Value =
        serde_json::from_str(rig_add["content"][0]["text"].as_str().unwrap()).unwrap();
    let rig_id = rig_json["item_id"].as_str().unwrap().to_string();
    assert_eq!(rig_json["bone_count"], 10);

    call_tool(
        &session,
        "rig2d_puppet_control",
        &json!({
            "action": "update_rig",
            "id": rig_id.clone(),
            "preset": "dance_bounce",
            "motion_speed": 1.3
        }),
    )
    .unwrap();

    let ik_prev = call_tool(
        &session,
        "rig2d_puppet_control",
        &json!({"action": "solve_ik_preview", "time_sec": 0.2}),
    )
    .unwrap();
    let ik_json: Value =
        serde_json::from_str(ik_prev["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(ik_json["bone_count"], 10);

    // 2. #38 offline_bgm_composer: preview + compose_and_place_bgm with bar markers
    let bgm_prev = call_tool(
        &session,
        "offline_bgm_composer",
        &json!({
            "action": "preview_arrangement",
            "genre": "cyber_synthwave",
            "duration_sec": 4.0
        }),
    )
    .unwrap();
    let bgm_prev_json: Value =
        serde_json::from_str(bgm_prev["content"][0]["text"].as_str().unwrap()).unwrap();
    assert!(bgm_prev_json["bar_count"].as_u64().unwrap() >= 2);

    let bgm_res = call_tool(
        &session,
        "offline_bgm_composer",
        &json!({
            "action": "compose_and_place_bgm",
            "genre": "lofi_chill",
            "duration_sec": 3.0,
            "audio_layer_index": 3,
            "add_bar_markers": true,
            "apply": true
        }),
    )
    .unwrap();
    let bgm_json: Value =
        serde_json::from_str(bgm_res["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(bgm_json["applied"], true);
    assert!(bgm_json["bar_markers_added"].as_u64().unwrap() >= 1);
    assert!(std::path::Path::new(bgm_json["wav_path"].as_str().unwrap()).exists());

    // 3. #39 speed_ramp_and_scene_detect: set bullet_time_dip on rig + detect hard cut
    let sr_res = call_tool(
        &session,
        "speed_ramp_and_scene_detect",
        &json!({
            "action": "set_speed_ramp",
            "id": rig_id,
            "preset": "bullet_time_dip",
            "enabled": true
        }),
    )
    .unwrap();
    let sr_json: Value =
        serde_json::from_str(sr_res["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(sr_json["applied"], true);

    // Create two contrasting full-screen colored shots for scene cut detection
    {
        let mut g = session.lock().unwrap();
        let sc = g.project.active_scene_mut().unwrap();
        sc.layers[0].items.push(TimelineItem::new(
            "RedShot",
            RationalTime::ZERO,
            RationalTime::from_seconds(1.5),
            ItemKind::Shape {
                shape: ShapeKind::Rectangle,
                width: 160.0,
                height: 90.0,
                color: RgbaColor {
                    r: 0.92,
                    g: 0.12,
                    b: 0.12,
                    a: 1.0,
                },
            },
        ));
        sc.layers[0].items.push(TimelineItem::new(
            "BlueShot",
            RationalTime::from_seconds(1.5),
            RationalTime::from_seconds(1.5),
            ItemKind::Shape {
                shape: ShapeKind::Rectangle,
                width: 160.0,
                height: 90.0,
                color: RgbaColor {
                    r: 0.08,
                    g: 0.22,
                    b: 0.94,
                    a: 1.0,
                },
            },
        ));
    }

    let cut_res = call_tool(
        &session,
        "speed_ramp_and_scene_detect",
        &json!({
            "action": "detect_scene_cuts",
            "sample_interval_sec": 0.25,
            "sensitivity": 0.6,
            "split_clips": false,
            "apply": true
        }),
    )
    .unwrap();
    let cut_json: Value =
        serde_json::from_str(cut_res["content"][0]["text"].as_str().unwrap()).unwrap();
    assert!(cut_json["cut_count"].as_u64().unwrap() >= 1);
    assert!(cut_json["applied_commands"].as_u64().unwrap() >= 1);

    // 4. #40 cinema_finishing_and_edl: set_finishing + export EDL + export FCPXML
    let fin_res = call_tool(
        &session,
        "cinema_finishing_and_edl",
        &json!({
            "action": "set_finishing",
            "preset": "kodak_35mm",
            "burn_in_timecode": true,
            "watermark_text": "V5 REVIEW"
        }),
    )
    .unwrap();
    let fin_json: Value =
        serde_json::from_str(fin_res["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(fin_json["applied"], true);
    assert_eq!(fin_json["config"]["burn_in_timecode"], true);

    let edl_path = tmp_dir.join("master.edl");
    let edl_res = call_tool(
        &session,
        "cinema_finishing_and_edl",
        &json!({
            "action": "export_edl_cmx3600",
            "output_path": edl_path.display().to_string()
        }),
    )
    .unwrap();
    let edl_json: Value =
        serde_json::from_str(edl_res["content"][0]["text"].as_str().unwrap()).unwrap();
    assert!(
        edl_json["edl"]
            .as_str()
            .unwrap()
            .contains("FCM: NON-DROP FRAME")
    );
    assert!(edl_path.exists());

    let xml_path = tmp_dir.join("master.fcpxml");
    let xml_res = call_tool(
        &session,
        "cinema_finishing_and_edl",
        &json!({
            "action": "export_fcpxml",
            "output_path": xml_path.display().to_string()
        }),
    )
    .unwrap();
    let xml_json: Value =
        serde_json::from_str(xml_res["content"][0]["text"].as_str().unwrap()).unwrap();
    assert!(
        xml_json["fcpxml"]
            .as_str()
            .unwrap()
            .contains("<fcpxml version=\"1.10\">")
    );
    assert!(xml_path.exists());

    let defs = tool_definitions();
    assert_eq!(defs.as_array().unwrap().len(), 82);
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn v60_mcp_tools_dataviz_vfxpaint_podcast_and_packager() {
    let tmp_dir = std::env::temp_dir().join(format!("ve_mcp_v60_test_{}", Uuid::new_v4()));
    std::fs::create_dir_all(&tmp_dir).unwrap();
    let proj = Project::new("V60CreatorStudioTest", ProjectSettings::default());
    let folder = ve_core::ProjectFolder::init(&tmp_dir, &proj).unwrap();
    let session = Arc::new(Mutex::new(EditorSession::with_folder(proj, folder)));

    // 1. #41 data_chart_motion_graphics: create_chart + import_csv
    let chart_res = call_tool(
        &session,
        "data_chart_motion_graphics",
        &json!({
            "action": "create_chart",
            "kind": "bar_chart",
            "title": "ANNUAL SALES",
            "unit_suffix": "K",
            "layer_index": 1,
            "duration_sec": 4.0
        }),
    )
    .unwrap();
    let chart_json: Value =
        serde_json::from_str(chart_res["content"][0]["text"].as_str().unwrap()).unwrap();
    let chart_id = chart_json["created_id"].as_str().unwrap().to_string();
    assert_eq!(chart_json["point_count"], 4);

    let csv_res = call_tool(
        &session,
        "data_chart_motion_graphics",
        &json!({
            "action": "import_csv",
            "item_id": chart_id,
            "csv_text": "label,value,color\nJAN,45,#22ccff\nFEB,72,#ff4488\nMAR,118,#44ee88\n"
        }),
    )
    .unwrap();
    let csv_json: Value =
        serde_json::from_str(csv_res["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(csv_json["updated"], true);
    assert_eq!(csv_json["point_count"], 3);

    // 2. #42 vfx_paint_and_inpaint: add_stroke (brush + wire_erase) -> get_paint -> clear_paint
    let paint1 = call_tool(
        &session,
        "vfx_paint_and_inpaint",
        &json!({
            "action": "add_stroke",
            "item_id": chart_id,
            "tool": "brush_glow",
            "points": [[0.1, 0.2], [0.9, 0.8]],
            "radius_px": 12.0,
            "color": "#ff3366",
            "animate_write_on": true
        }),
    )
    .unwrap();
    let p1_json: Value =
        serde_json::from_str(paint1["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(p1_json["applied"], true);
    assert_eq!(p1_json["stroke_count"], 1);

    let paint2 = call_tool(
        &session,
        "vfx_paint_and_inpaint",
        &json!({
            "action": "add_stroke",
            "item_id": chart_id,
            "tool": "wire_erase_inpaint",
            "points": [[0.2, 0.5], [0.8, 0.5]],
            "radius_px": 8.0
        }),
    )
    .unwrap();
    let p2_json: Value =
        serde_json::from_str(paint2["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(p2_json["stroke_count"], 2);

    // 3. #43 podcast_auto_director: write 2 speaker WAVs, analyze_and_direct + level_wav
    let sr = 24_000u32;
    let frames = sr as usize * 3;
    let mut host_a = vec![0.0f32; frames * 2];
    let mut host_b = vec![0.0f32; frames * 2];
    for i in 0..frames {
        let t = i as f32 / sr as f32;
        if t < 1.3 {
            let v = (std::f32::consts::TAU * 220.0 * t).sin() * 0.14;
            host_a[i * 2] = v;
            host_a[i * 2 + 1] = v;
        }
        if (1.5..2.8).contains(&t) {
            let v = (std::f32::consts::TAU * 170.0 * t).sin() * 0.48;
            host_b[i * 2] = v;
            host_b[i * 2 + 1] = v;
        }
    }
    let wav_a = tmp_dir.join("host_a.wav");
    let wav_b = tmp_dir.join("host_b.wav");
    let wav_leveled = tmp_dir.join("host_a_leveled.wav");
    write_stereo_wav_file(&wav_a, &host_a, sr).unwrap();
    write_stereo_wav_file(&wav_b, &host_b, sr).unwrap();

    let pod_res = call_tool(
            &session,
            "podcast_auto_director",
            &json!({
                "action": "analyze_and_direct",
                "speakers": [
                    { "name": "Host A", "audio_layer_index": 2, "camera_layer_index": 0, "wav_path": wav_a.display().to_string() },
                    { "name": "Host B", "audio_layer_index": 3, "camera_layer_index": 1, "wav_path": wav_b.display().to_string() }
                ],
                "min_turn_sec": 0.3,
                "target_lufs": -16.0,
                "apply": true
            }),
        )
        .unwrap();
    let pod_json: Value =
        serde_json::from_str(pod_res["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(pod_json["applied"], true);
    assert_eq!(pod_json["turn_count"], 2);

    let lvl_res = call_tool(
        &session,
        "podcast_auto_director",
        &json!({
            "action": "level_wav",
            "input_wav_path": wav_a.display().to_string(),
            "output_wav_path": wav_leveled.display().to_string(),
            "target_lufs": -16.0
        }),
    )
    .unwrap();
    let lvl_json: Value =
        serde_json::from_str(lvl_res["content"][0]["text"].as_str().unwrap()).unwrap();
    assert!(wav_leveled.exists());
    assert!(lvl_json["after_lufs"].as_f64().unwrap() > lvl_json["before_lufs"].as_f64().unwrap());

    // 4. #44 multi_platform_packager: create_platform_variants + export_delivery_bundle
    let var_res = call_tool(
        &session,
        "multi_platform_packager",
        &json!({
            "action": "create_platform_variants",
            "platforms": ["youtube_16x9", "shorts_reels_9x16", "square_feed_1x1"]
        }),
    )
    .unwrap();
    let var_json: Value =
        serde_json::from_str(var_res["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(var_json["created_count"], 3);

    let bundle_dir = tmp_dir.join("delivery_bundle");
    let bdl_res = call_tool(
        &session,
        "multi_platform_packager",
        &json!({
            "action": "export_delivery_bundle",
            "output_dir": bundle_dir.display().to_string(),
            "platforms": ["youtube_16x9", "shorts_reels_9x16"],
            "base_title": "V6 Launch Episode",
            "tags": ["rust", "video", "v6"]
        }),
    )
    .unwrap();
    let bdl_json: Value =
        serde_json::from_str(bdl_res["content"][0]["text"].as_str().unwrap()).unwrap();
    assert!(bdl_json["artifact_count"].as_u64().unwrap() >= 8);
    assert!(bundle_dir.join("manifest.json").exists());
    assert!(bundle_dir.join("thumbnail_1280x720.png").exists());
    assert!(bundle_dir.join("interchange/master.edl").exists());
    assert!(bundle_dir.join("interchange/master.fcpxml").exists());
    assert!(bundle_dir.join("qc/broadcast_qc_report.json").exists());
    assert!(bundle_dir.join("subtitles/shorts_reels_9x16.srt").exists());

    let defs = tool_definitions();
    assert_eq!(defs.as_array().unwrap().len(), 82);
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn v70_mcp_tools_hub_catalog_gpu_pool_index_conform_and_ab() {
    let mut proj = Project::new("V70UnifiedStudioTest", ProjectSettings::default());
    let proxy_id = Uuid::new_v4();
    let orig_id = Uuid::new_v4();
    proj.assets.push(AssetRef {
        id: proxy_id,
        name: "B003_C015_720p_proxy.mp4".into(),
        kind: MediaKind::Video,
        relative_path: None,
        fallback_absolute_path: None,
        collected: false,
    });
    proj.assets.push(AssetRef {
        id: orig_id,
        name: "B003_C015_4k_master.mov".into(),
        kind: MediaKind::Video,
        relative_path: None,
        fallback_absolute_path: None,
        collected: false,
    });
    {
        let scene = proj.active_scene_mut().unwrap();
        scene.layers[0].items.push(TimelineItem::new(
            "ProxyClip",
            RationalTime::ZERO,
            RationalTime::from_seconds(3.0),
            ItemKind::Video {
                asset_id: proxy_id,
                source_offset: RationalTime::from_seconds(2.0),
                volume: 1.0,
            },
        ));
        scene.layers[1].items.push(TimelineItem::new(
            "HeroShape",
            RationalTime::ZERO,
            RationalTime::from_seconds(3.0),
            ItemKind::Shape {
                shape: ShapeKind::Rectangle,
                width: 300.0,
                height: 180.0,
                color: RgbaColor {
                    r: 0.2,
                    g: 0.5,
                    b: 0.9,
                    a: 1.0,
                },
            },
        ));
    }
    let session = Arc::new(Mutex::new(EditorSession::new(proj)));

    // 1. #45 studio_hub_and_command_palette: list_hubs & search_palette
    let hubs_res = call_tool(
        &session,
        "studio_hub_and_command_palette",
        &json!({ "action": "list_hubs" }),
    )
    .unwrap();
    let hubs_json: Value =
        serde_json::from_str(hubs_res["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(hubs_json["hub_count"], 6);
    assert!(hubs_json["total_palette_commands"].as_u64().unwrap() >= 45);

    let search_res = call_tool(
        &session,
        "studio_hub_and_command_palette",
        &json!({ "action": "search_palette", "query": "コンフォーム", "limit": 5 }),
    )
    .unwrap();
    let search_json: Value =
        serde_json::from_str(search_res["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(search_json["hits"][0]["id"], "hub.edit.conform_index");

    // 2. #46 mcp_catalog_introspect: summary, search_tools, verify_schemas
    let cat_res = call_tool(
        &session,
        "mcp_catalog_introspect",
        &json!({ "action": "summary" }),
    )
    .unwrap();
    let cat_json: Value =
        serde_json::from_str(cat_res["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(cat_json["domain_count"], 7);
    assert_eq!(cat_json["total_tools"], 82);

    let verify_res = call_tool(
        &session,
        "mcp_catalog_introspect",
        &json!({ "action": "verify_schemas" }),
    )
    .unwrap();
    let verify_json: Value =
        serde_json::from_str(verify_res["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(verify_json["valid"], true);
    assert_eq!(verify_json["unique_tool_count"], 82);

    // 3. #47 gpu_v70_frame_pool: status, benchmark (custom lightweight), run_fast_blend_aces
    let gpu_st_res = call_tool(
        &session,
        "gpu_v70_frame_pool",
        &json!({ "action": "status" }),
    )
    .unwrap();
    let gpu_st: Value =
        serde_json::from_str(gpu_st_res["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(gpu_st["blend_mask_shader_valid"], true);
    assert_eq!(gpu_st["aces_color_shader_valid"], true);

    let gpu_bench_res = call_tool(
        &session,
        "gpu_v70_frame_pool",
        &json!({ "action": "benchmark", "preset_resolution": "custom", "width": 128, "height": 72, "iterations": 3 }),
    )
    .unwrap();
    let gpu_bench: Value =
        serde_json::from_str(gpu_bench_res["content"][0]["text"].as_str().unwrap()).unwrap();
    assert!(gpu_bench["pool_reuses"].as_u64().unwrap() >= 2);

    let fast_res = call_tool(
        &session,
        "gpu_v70_frame_pool",
        &json!({ "action": "run_fast_blend_aces", "blend_mode": "screen", "mask_shape": "soft_ellipse" }),
    )
    .unwrap();
    let fast_json: Value =
        serde_json::from_str(fast_res["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(fast_json["width"], 320);

    // 4. #48 timeline_index_conform_ab: 10k interval index benchmark + TC conform + A/B compare & PNG preview
    let bench10k_res = call_tool(
        &session,
        "timeline_index_conform_ab",
        &json!({ "action": "benchmark_10k_index", "clip_count": 10000, "query_count": 50 }),
    )
    .unwrap();
    let bench10k: Value =
        serde_json::from_str(bench10k_res["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(bench10k["clip_count"], 10000);
    assert_eq!(bench10k["exact_parity_verified"], true);

    call_tool(
        &session,
        "timeline_index_conform_ab",
        &json!({
            "action": "set_asset_conform_meta",
            "asset_id": proxy_id.to_string(),
            "reel_name": "B003_C015",
            "start_timecode": "01:00:05:00",
            "is_proxy_media": true
        }),
    )
    .unwrap();
    call_tool(
        &session,
        "timeline_index_conform_ab",
        &json!({
            "action": "set_asset_conform_meta",
            "asset_id": orig_id.to_string(),
            "reel_name": "B003_C015",
            "start_timecode": "01:00:00:00",
            "is_proxy_media": false
        }),
    )
    .unwrap();

    let conf_res = call_tool(
        &session,
        "timeline_index_conform_ab",
        &json!({ "action": "conform_timeline", "direction": "proxy_to_original", "apply": true }),
    )
    .unwrap();
    let conf_json: Value =
        serde_json::from_str(conf_res["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(conf_json["applied"], true);
    assert_eq!(conf_json["action_count"], 1);
    {
        let g = session.lock().unwrap();
        if let ItemKind::Video {
            asset_id,
            source_offset,
            ..
        } = &g.project.active_scene().unwrap().layers[0].items[0].kind
        {
            assert_eq!(*asset_id, orig_id);
            assert!((source_offset.as_seconds() - 7.0).abs() < 1e-4);
        } else {
            panic!("expected video item after conform");
        }
    }

    call_tool(
        &session,
        "timeline_index_conform_ab",
        &json!({
            "action": "set_ab_compare",
            "ab_mode": "vertical_split",
            "ab_reference": "ungraded_bypass",
            "split_ratio": 0.45
        }),
    )
    .unwrap();
    let ab_png = call_tool(
        &session,
        "timeline_index_conform_ab",
        &json!({ "action": "render_ab_preview", "time_sec": 0.5, "scale": 0.2 }),
    )
    .unwrap();
    assert_eq!(ab_png["content"][0]["mimeType"], "image/png");
    assert!(!ab_png["content"][0]["data"].as_str().unwrap().is_empty());

    let defs = tool_definitions();
    assert_eq!(defs.as_array().unwrap().len(), 82);
}
