//! v7.0 #46: 内蔵 MCP ドメインモジュール — Color Grading, ACEScg/OCIO & Cinema Finishing

#[allow(unused_imports)]
use super::*;

pub(super) const DOMAIN_ID: &str = "color_finishing";
pub(super) const DOMAIN_LABEL: &str = "Color Grading, ACEScg/OCIO & Cinema Finishing";

pub(super) fn definitions() -> Vec<Value> {
    let mut out = Vec::new();
    if let Value::Array(items) = json!([
        {
            "name": "set_color_grade",
            "description": "アクティブシーンの 3-Way カラーグレーディング（露出/コントラスト/彩度/色温度/ティント/Lift/Gamma/Gain）および .cube 3D LUT を設定・リセットする（Undo対応）",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "enabled": { "type": "boolean" },
                    "reset": { "type": "boolean", "default": false },
                    "exposure": { "type": "number" },
                    "contrast": { "type": "number" },
                    "saturation": { "type": "number" },
                    "temperature": { "type": "number" },
                    "tint": { "type": "number" },
                    "lift": { "type": "array", "items": { "type": "number" } },
                    "gamma": { "type": "array", "items": { "type": "number" } },
                    "gain": { "type": "array", "items": { "type": "number" } },
                    "lut_path": { "type": "string" },
                    "lut_intensity": { "type": "number" },
                    "emit_sample_luts": { "type": "boolean", "default": false }
                }
            }
        },
        {
            "name": "pro_color_match_and_grade",
            "description": "v2.1 #25: RGB/マスタートーンカーブ（Fritsch-Carlson単調スプライン）・HSL二次色補正クオリファイア（マット可視化付）・ショット間自動カラーマッチを実行する",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["set_curves", "set_hsl_qualifier", "auto_match_timecodes"]
                    },
                    "enabled": { "type": "boolean" },
                    "preset": {
                        "type": "string",
                        "enum": ["identity", "s_curve", "film_fade", "warm_cinema"]
                    },
                    "master": { "type": "array" },
                    "red": { "type": "array" },
                    "green": { "type": "array" },
                    "blue": { "type": "array" },
                    "hue_center_deg": { "type": "number" },
                    "hue_width_deg": { "type": "number" },
                    "sat_min": { "type": "number" },
                    "luma_min": { "type": "number" },
                    "softness": { "type": "number" },
                    "hue_shift_deg": { "type": "number" },
                    "sat_scale": { "type": "number" },
                    "luma_offset": { "type": "number" },
                    "show_matte_overlay": { "type": "boolean" },
                    "reference_time_sec": { "type": "number" },
                    "target_time_sec": { "type": "number" },
                    "apply": { "type": "boolean" }
                }
            }
        },
        {
            "name": "aces_color_management",
            "description": "v4.0 #33: ACEScg (AP1) / OpenColorIO v2 プロカラー管理（Sony S-Log3・Panasonic V-Log・ARRI LogC3 IDT、ACES 1.3 ガマット圧縮、ACES Fitted / AgX / Reinhard トーンマップ、Display P3 / Rec.2020 ODT、`.ocio` YAML 書き出し）を制御する",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["get_config", "set_config", "export_ocio"]
                    },
                    "enabled": { "type": "boolean" },
                    "input_idt": {
                        "type": "string",
                        "enum": ["linear_srgb", "aces_cg", "display_p3", "rec2020", "s_log3", "v_log", "log_c3"]
                    },
                    "working_space": {
                        "type": "string",
                        "enum": ["linear_srgb", "aces_cg", "display_p3", "rec2020"]
                    },
                    "output_odt": {
                        "type": "string",
                        "enum": ["linear_srgb", "display_p3", "rec2020"]
                    },
                    "tone_mapper": {
                        "type": "string",
                        "enum": ["none", "aces_fitted", "ag_x", "reinhard_extended"]
                    },
                    "exposure_stops": { "type": "number" },
                    "gamut_compress": { "type": "boolean" },
                    "output_path": { "type": "string" }
                }
            }
        },
        {
            "name": "cinema_finishing_and_edl",
            "description": "v5.0 #40: プロ・シネマフィニッシング（35mm ハレーション・アナモルフィックフレア・フィルムグレイン・ビネット・レターボックス・SMPTE 透かしバーンイン）の制御および業界標準 CMX 3600 EDL / Final Cut Pro XML v1.10 (FCPXML) 書き出しを行う",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["get_finishing", "set_finishing", "export_edl_cmx3600", "export_fcpxml"]
                    },
                    "enabled": { "type": "boolean" },
                    "preset": {
                        "type": "string",
                        "enum": ["kodak_35mm", "scifi_anamorphic", "agency_review_burnin"]
                    },
                    "halation_strength": { "type": "number" },
                    "halation_threshold": { "type": "number" },
                    "anamorphic_flare": { "type": "number" },
                    "film_grain": { "type": "number" },
                    "vignette": { "type": "number" },
                    "letterbox": {
                        "type": "string",
                        "enum": ["none", "cinema239", "flat185", "academy43"]
                    },
                    "burn_in_timecode": { "type": "boolean" },
                    "watermark_text": { "type": "string" },
                    "output_path": { "type": "string" }
                }
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
        "set_color_grade"
            | "pro_color_match_and_grade"
            | "aces_color_management"
            | "cinema_finishing_and_edl"
    ) {
        return None;
    }
    Some(call_inner(guard, fps, name, args))
}

fn call_inner(
    guard: &mut crate::session::EditorSession,
    _fps: FrameRate,
    name: &str,
    args: &Value,
) -> Result<Value, String> {
    match name {
        "set_color_grade" => {
            let mut emitted_luts: Vec<String> = Vec::new();
            if args
                .get("emit_sample_luts")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                let dir = guard
                    .folder
                    .as_ref()
                    .map(|f| f.luts_dir())
                    .unwrap_or_else(|| std::env::temp_dir().join("ve_mcp_luts"));
                let paths = emit_sample_cube_luts(&dir)?;
                emitted_luts = paths.into_iter().map(|p| p.display().to_string()).collect();
            }

            let mut grade = if args.get("reset").and_then(Value::as_bool).unwrap_or(false) {
                SceneColorGrade::default()
            } else {
                guard
                    .project
                    .active_scene()
                    .map(|s| s.color_grade.clone())
                    .ok_or("no active scene")?
            };
            if let Some(en) = args.get("enabled").and_then(Value::as_bool) {
                grade.enabled = en;
            } else if !args.get("reset").and_then(Value::as_bool).unwrap_or(false) {
                grade.enabled = true;
            }
            if args.get("exposure").is_some() {
                grade.exposure = finite_clamped(args.get("exposure"), 0.0, -3.0, 3.0) as f32;
            }
            if args.get("contrast").is_some() {
                grade.contrast = finite_clamped(args.get("contrast"), 1.0, 0.2, 2.5) as f32;
            }
            if args.get("saturation").is_some() {
                grade.saturation = finite_clamped(args.get("saturation"), 1.0, 0.0, 2.5) as f32;
            }
            if args.get("temperature").is_some() {
                grade.temperature = finite_clamped(args.get("temperature"), 0.0, -1.0, 1.0) as f32;
            }
            if args.get("tint").is_some() {
                grade.tint = finite_clamped(args.get("tint"), 0.0, -1.0, 1.0) as f32;
            }
            let parse_vec3 = |key: &str, lo: f64, hi: f64| -> Option<[f32; 3]> {
                let arr = args.get(key)?.as_array()?;
                if arr.len() != 3 {
                    return None;
                }
                Some([
                    finite_clamped(arr.first(), 0.0, lo, hi) as f32,
                    finite_clamped(arr.get(1), 0.0, lo, hi) as f32,
                    finite_clamped(arr.get(2), 0.0, lo, hi) as f32,
                ])
            };
            if let Some(v) = parse_vec3("lift", -0.5, 0.5) {
                grade.lift = v;
            }
            if let Some(v) = parse_vec3("gamma", -0.5, 0.5) {
                grade.gamma = v;
            }
            if let Some(v) = parse_vec3("gain", 0.2, 2.5) {
                grade.gain = v;
            }
            if let Some(lp) = args.get("lut_path").and_then(Value::as_str) {
                grade.lut_path = if lp.trim().is_empty() {
                    None
                } else {
                    Some(lp.trim().to_string())
                };
            }
            if args.get("lut_intensity").is_some() {
                grade.lut_intensity =
                    finite_clamped(args.get("lut_intensity"), 1.0, 0.0, 1.0) as f32;
            }

            let sess = &mut *guard;
            sess.bus
                .execute(
                    &mut sess.project,
                    EditCommand::SetSceneColorGrade {
                        grade: grade.clone(),
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": json!({
                        "color_grade": grade,
                        "emitted_luts": emitted_luts
                    }).to_string()
                }]
            }))
        }
        "pro_color_match_and_grade" => {
            let s = &mut *guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing 'action'".to_string())?;
            let parse_curve_pts = |key: &str| -> Option<Vec<[f32; 2]>> {
                let arr = args.get(key)?.as_array()?;
                let pts: Vec<[f32; 2]> = arr
                    .iter()
                    .take(16)
                    .filter_map(|p| {
                        let pair = p.as_array()?;
                        if pair.len() < 2 {
                            return None;
                        }
                        let x = pair[0].as_f64()?;
                        let y = pair[1].as_f64()?;
                        if !x.is_finite() || !y.is_finite() {
                            return None;
                        }
                        Some([x.clamp(0.0, 1.0) as f32, y.clamp(0.0, 1.0) as f32])
                    })
                    .collect();
                if pts.len() >= 2 { Some(pts) } else { None }
            };

            match action {
                "set_curves" => {
                    let mut grade = s
                        .project
                        .active_scene()
                        .ok_or_else(|| "no active scene".to_string())?
                        .color_grade
                        .clone();
                    if let Some(preset) = args.get("preset").and_then(Value::as_str) {
                        grade.curves = match preset {
                            "s_curve" => ColorCurves::preset_s_curve(),
                            "film_fade" => ColorCurves::preset_film_fade(),
                            "warm_cinema" => ColorCurves::preset_warm_cinema(),
                            _ => ColorCurves::default(),
                        };
                    }
                    if let Some(en) = args.get("enabled").and_then(Value::as_bool) {
                        grade.curves.enabled = en;
                    } else if args.get("preset").is_none() {
                        grade.curves.enabled = true;
                    }
                    if let Some(pts) = parse_curve_pts("master") {
                        grade.curves.master = pts;
                    }
                    if let Some(pts) = parse_curve_pts("red") {
                        grade.curves.red = pts;
                    }
                    if let Some(pts) = parse_curve_pts("green") {
                        grade.curves.green = pts;
                    }
                    if let Some(pts) = parse_curve_pts("blue") {
                        grade.curves.blue = pts;
                    }
                    if grade.curves.enabled {
                        grade.enabled = true;
                    }
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetSceneColorGrade {
                                grade: grade.clone(),
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": serde_json::to_string(&grade.curves).unwrap_or_default() }]
                    }))
                }
                "set_hsl_qualifier" => {
                    let mut grade = s
                        .project
                        .active_scene()
                        .ok_or_else(|| "no active scene".to_string())?
                        .color_grade
                        .clone();
                    let mut q = grade.hsl_qualifier;
                    q.enabled = args.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                    if args.get("hue_center_deg").is_some() {
                        q.hue_center_deg = finite_clamped(
                            args.get("hue_center_deg"),
                            f64::from(q.hue_center_deg),
                            0.0,
                            360.0,
                        ) as f32;
                    }
                    if args.get("hue_width_deg").is_some() {
                        q.hue_width_deg = finite_clamped(
                            args.get("hue_width_deg"),
                            f64::from(q.hue_width_deg),
                            5.0,
                            120.0,
                        ) as f32;
                    }
                    if args.get("sat_min").is_some() {
                        q.sat_min =
                            finite_clamped(args.get("sat_min"), f64::from(q.sat_min), 0.0, 1.0)
                                as f32;
                    }
                    if args.get("luma_min").is_some() {
                        q.luma_min =
                            finite_clamped(args.get("luma_min"), f64::from(q.luma_min), 0.0, 1.0)
                                as f32;
                    }
                    if args.get("softness").is_some() {
                        q.softness =
                            finite_clamped(args.get("softness"), f64::from(q.softness), 0.02, 1.0)
                                as f32;
                    }
                    if args.get("hue_shift_deg").is_some() {
                        q.hue_shift_deg = finite_clamped(
                            args.get("hue_shift_deg"),
                            f64::from(q.hue_shift_deg),
                            -180.0,
                            180.0,
                        ) as f32;
                    }
                    if args.get("sat_scale").is_some() {
                        q.sat_scale =
                            finite_clamped(args.get("sat_scale"), f64::from(q.sat_scale), 0.0, 2.5)
                                as f32;
                    }
                    if args.get("luma_offset").is_some() {
                        q.luma_offset = finite_clamped(
                            args.get("luma_offset"),
                            f64::from(q.luma_offset),
                            -0.5,
                            0.5,
                        ) as f32;
                    }
                    if let Some(m) = args.get("show_matte_overlay").and_then(Value::as_bool) {
                        q.show_matte_overlay = m;
                    }
                    grade.hsl_qualifier = q;
                    if q.enabled {
                        grade.enabled = true;
                    }
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetSceneColorGrade {
                                grade: grade.clone(),
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": serde_json::to_string(&grade.hsl_qualifier).unwrap_or_default() }]
                    }))
                }
                "auto_match_timecodes" => {
                    let ref_sec =
                        finite_clamped(args.get("reference_time_sec"), 0.0, 0.0, 86_400.0);
                    let tgt_sec = finite_clamped(
                        args.get("target_time_sec"),
                        s.playhead.as_seconds(),
                        0.0,
                        86_400.0,
                    );
                    let apply = args.get("apply").and_then(Value::as_bool).unwrap_or(true);
                    let scene_id = s.project.active_scene_id;
                    let mut rg = RenderGraph::new();
                    rg.set_project_folder(s.folder.clone());
                    let ref_img = rg
                        .render_project_scene_scaled(
                            &s.project,
                            scene_id,
                            RationalTime::from_seconds(ref_sec),
                            320,
                            180,
                            false,
                        )
                        .ok_or_else(|| "failed to render reference frame".to_string())?;
                    let tgt_img = rg
                        .render_project_scene_scaled(
                            &s.project,
                            scene_id,
                            RationalTime::from_seconds(tgt_sec),
                            320,
                            180,
                            false,
                        )
                        .ok_or_else(|| "failed to render target frame".to_string())?;

                    let matched = compute_auto_color_match(&ref_img, &tgt_img);
                    let mut next_grade = s
                        .project
                        .active_scene()
                        .ok_or_else(|| "no active scene".to_string())?
                        .color_grade
                        .clone();
                    next_grade.enabled = true;
                    next_grade.lift = matched.lift;
                    next_grade.gain = matched.gain;
                    next_grade.saturation = matched.saturation;

                    if apply {
                        s.bus
                            .execute(
                                &mut s.project,
                                EditCommand::SetSceneColorGrade {
                                    grade: next_grade.clone(),
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
                                "applied": apply,
                                "reference_time_sec": ref_sec,
                                "target_time_sec": tgt_sec,
                                "matched_grade": next_grade
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown pro_color_match_and_grade action: {other}")),
            }
        }
        "aces_color_management" => {
            let s = &mut *guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing 'action'".to_string())?;
            let parse_cs = |k: &str, fallback: ColorSpace| -> ColorSpace {
                match args.get(k).and_then(Value::as_str) {
                    Some("linear_srgb") => ColorSpace::LinearSrgb,
                    Some("aces_cg") => ColorSpace::AcesCg,
                    Some("display_p3") => ColorSpace::DisplayP3,
                    Some("rec2020") => ColorSpace::Rec2020,
                    Some("s_log3") => ColorSpace::SLog3,
                    Some("v_log") => ColorSpace::VLog,
                    Some("log_c3") => ColorSpace::LogC3,
                    _ => fallback,
                }
            };
            let parse_tm = |k: &str, fallback: ToneMapper| -> ToneMapper {
                match args.get(k).and_then(Value::as_str) {
                    Some("none") => ToneMapper::None,
                    Some("aces_fitted") => ToneMapper::AcesFitted,
                    Some("ag_x") => ToneMapper::AgX,
                    Some("reinhard_extended") => ToneMapper::ReinhardExtended,
                    _ => fallback,
                }
            };
            match action {
                "get_config" => {
                    let cfg = s
                        .project
                        .active_scene()
                        .map(|sc| sc.color_management)
                        .ok_or_else(|| "no active scene".to_string())?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": serde_json::to_string(&cfg).unwrap_or_default() }]
                    }))
                }
                "set_config" => {
                    let cur = s
                        .project
                        .active_scene()
                        .map(|sc| sc.color_management)
                        .ok_or_else(|| "no active scene".to_string())?;
                    let next = ColorManagementConfig {
                        enabled: args
                            .get("enabled")
                            .and_then(Value::as_bool)
                            .unwrap_or(cur.enabled),
                        input_idt: parse_cs("input_idt", cur.input_idt),
                        working_space: parse_cs("working_space", cur.working_space),
                        output_odt: parse_cs("output_odt", cur.output_odt),
                        tone_mapper: parse_tm("tone_mapper", cur.tone_mapper),
                        exposure_stops: finite_clamped(
                            args.get("exposure_stops"),
                            f64::from(cur.exposure_stops),
                            -4.0,
                            4.0,
                        ) as f32,
                        gamut_compress: args
                            .get("gamut_compress")
                            .and_then(Value::as_bool)
                            .unwrap_or(cur.gamut_compress),
                    };
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetColorManagement { config: next },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({ "applied": true, "config": next }).to_string()
                        }]
                    }))
                }
                "export_ocio" => {
                    let cfg = s
                        .project
                        .active_scene()
                        .map(|sc| sc.color_management)
                        .ok_or_else(|| "no active scene".to_string())?;
                    let yaml = export_ocio_config_yaml(&cfg);
                    let saved_path =
                        if let Some(p) = args.get("output_path").and_then(Value::as_str) {
                            let out_p = PathBuf::from(p);
                            if let Some(parent) = out_p.parent()
                                && !parent.as_os_str().is_empty()
                            {
                                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                            }
                            std::fs::write(&out_p, &yaml).map_err(|e| e.to_string())?;
                            Some(out_p.display().to_string())
                        } else {
                            None
                        };
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({ "ocio_yaml": yaml, "saved_path": saved_path }).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown aces_color_management action: {other}")),
            }
        }
        "cinema_finishing_and_edl" => {
            let s = &mut *guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing 'action'".to_string())?;
            match action {
                "get_finishing" => {
                    let cfg = s
                        .project
                        .active_scene()
                        .map(|sc| sc.cinema_finishing.clone())
                        .ok_or_else(|| "no active scene".to_string())?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": serde_json::to_string(&cfg).unwrap_or_default() }]
                    }))
                }
                "set_finishing" => {
                    let mut cfg = match args.get("preset").and_then(Value::as_str) {
                        Some("kodak_35mm") => CinemaFinishingConfig::preset_kodak_35mm(),
                        Some("scifi_anamorphic") => {
                            CinemaFinishingConfig::preset_scifi_anamorphic()
                        }
                        Some("agency_review_burnin") => {
                            CinemaFinishingConfig::preset_agency_review_burnin()
                        }
                        _ => s
                            .project
                            .active_scene()
                            .map(|sc| sc.cinema_finishing.clone())
                            .ok_or_else(|| "no active scene".to_string())?,
                    };
                    if let Some(en) = args.get("enabled").and_then(Value::as_bool) {
                        cfg.enabled = en;
                    }
                    if args.get("halation_strength").is_some() {
                        cfg.halation_strength = finite_clamped(
                            args.get("halation_strength"),
                            f64::from(cfg.halation_strength),
                            0.0,
                            1.5,
                        ) as f32;
                    }
                    if args.get("halation_threshold").is_some() {
                        cfg.halation_threshold = finite_clamped(
                            args.get("halation_threshold"),
                            f64::from(cfg.halation_threshold),
                            0.3,
                            1.0,
                        ) as f32;
                    }
                    if args.get("anamorphic_flare").is_some() {
                        cfg.anamorphic_flare = finite_clamped(
                            args.get("anamorphic_flare"),
                            f64::from(cfg.anamorphic_flare),
                            0.0,
                            1.5,
                        ) as f32;
                    }
                    if args.get("film_grain").is_some() {
                        cfg.film_grain = finite_clamped(
                            args.get("film_grain"),
                            f64::from(cfg.film_grain),
                            0.0,
                            0.5,
                        ) as f32;
                    }
                    if args.get("vignette").is_some() {
                        cfg.vignette =
                            finite_clamped(args.get("vignette"), f64::from(cfg.vignette), 0.0, 1.0)
                                as f32;
                    }
                    if let Some(lb) = args.get("letterbox").and_then(Value::as_str) {
                        cfg.letterbox = match lb {
                            "cinema239" => LetterboxAspect::Cinema239,
                            "flat185" => LetterboxAspect::Flat185,
                            "academy43" => LetterboxAspect::Academy43,
                            _ => LetterboxAspect::None,
                        };
                    }
                    if let Some(tc) = args.get("burn_in_timecode").and_then(Value::as_bool) {
                        cfg.burn_in_timecode = tc;
                    }
                    if let Some(wm) = args.get("watermark_text").and_then(Value::as_str) {
                        cfg.watermark_text = wm.to_string();
                    }
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetCinemaFinishing {
                                config: cfg.clone(),
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({ "applied": true, "config": cfg }).to_string()
                        }]
                    }))
                }
                "export_edl_cmx3600" => {
                    let edl = export_cmx3600_edl(&s.project);
                    let saved_path =
                        if let Some(p) = args.get("output_path").and_then(Value::as_str) {
                            let out_p = PathBuf::from(p);
                            if let Some(parent) = out_p.parent()
                                && !parent.as_os_str().is_empty()
                            {
                                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                            }
                            std::fs::write(&out_p, &edl).map_err(|e| e.to_string())?;
                            Some(out_p.display().to_string())
                        } else {
                            None
                        };
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({ "edl": edl, "saved_path": saved_path }).to_string()
                        }]
                    }))
                }
                "export_fcpxml" => {
                    let xml = export_fcpxml_v10(&s.project);
                    let saved_path =
                        if let Some(p) = args.get("output_path").and_then(Value::as_str) {
                            let out_p = PathBuf::from(p);
                            if let Some(parent) = out_p.parent()
                                && !parent.as_os_str().is_empty()
                            {
                                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                            }
                            std::fs::write(&out_p, &xml).map_err(|e| e.to_string())?;
                            Some(out_p.display().to_string())
                        } else {
                            None
                        };
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({ "fcpxml": xml, "saved_path": saved_path }).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown cinema_finishing_and_edl action: {other}")),
            }
        }
        other => Err(format!("unknown tool: {other}")),
    }
}
