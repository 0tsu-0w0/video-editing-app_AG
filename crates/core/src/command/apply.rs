//! `EditCommand` を `Project` へ適用する決定論的エグゼキュータ

use super::*;

fn find_item_ref(project: &Project, item_id: ItemId) -> Result<&TimelineItem, CommandError> {
    let scene = project.active_scene().ok_or(CommandError::NoActiveScene)?;
    for layer in &scene.layers {
        if let Some(item) = layer.items.iter().find(|i| i.id == item_id) {
            return Ok(item);
        }
    }
    Err(CommandError::ItemNotFound(item_id))
}

fn find_item_mut(
    project: &mut Project,
    item_id: ItemId,
) -> Result<&mut TimelineItem, CommandError> {
    let scene = project
        .active_scene_mut()
        .ok_or(CommandError::NoActiveScene)?;
    for layer in &mut scene.layers {
        if let Some(item) = layer.items.iter_mut().find(|i| i.id == item_id) {
            return Ok(item);
        }
    }
    Err(CommandError::ItemNotFound(item_id))
}

pub(super) fn apply_command(
    project: &mut Project,
    command: &EditCommand,
) -> Result<(), CommandError> {
    match command {
        EditCommand::AddObject { layer_index, item } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            let layer = scene
                .layers
                .get_mut(*layer_index)
                .ok_or(CommandError::InvalidLayer(*layer_index))?;
            layer.items.push(item.clone());
            Ok(())
        }
        EditCommand::DeleteObject { item_id } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            for layer in &mut scene.layers {
                if let Some(pos) = layer.items.iter().position(|i| i.id == *item_id) {
                    layer.items.remove(pos);
                    return Ok(());
                }
            }
            Err(CommandError::ItemNotFound(*item_id))
        }
        EditCommand::MoveObject {
            item_id,
            target_layer_index,
            new_start,
        } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            if *target_layer_index >= scene.layers.len() {
                return Err(CommandError::InvalidLayer(*target_layer_index));
            }
            let mut extracted = None;
            for layer in &mut scene.layers {
                if let Some(pos) = layer.items.iter().position(|i| i.id == *item_id) {
                    extracted = Some(layer.items.remove(pos));
                    break;
                }
            }
            let mut item = extracted.ok_or(CommandError::ItemNotFound(*item_id))?;
            item.start = (*new_start).max(RationalTime::ZERO);
            scene.layers[*target_layer_index].items.push(item);
            Ok(())
        }
        EditCommand::TrimObject {
            item_id,
            new_start,
            new_duration,
        } => {
            let item = find_item_mut(project, *item_id)?;
            item.start = (*new_start).max(RationalTime::ZERO);
            item.duration = (*new_duration).max(RationalTime::from_ticks(1));
            Ok(())
        }
        EditCommand::SplitObject {
            item_id,
            split_at,
            new_right_id,
        } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            for layer in &mut scene.layers {
                if let Some(pos) = layer.items.iter().position(|i| i.id == *item_id) {
                    let left = &mut layer.items[pos];
                    if *split_at <= left.start || *split_at >= left.end() {
                        return Err(CommandError::InvalidSplitPosition(*split_at));
                    }
                    let left_dur = *split_at - left.start;
                    let right_dur = left.end() - *split_at;
                    let mut right = left.clone();
                    right.id = *new_right_id;
                    right.start = *split_at;
                    right.duration = right_dur;
                    match &mut right.kind {
                        ItemKind::Video { source_offset, .. }
                        | ItemKind::Audio { source_offset, .. } => {
                            *source_offset = *source_offset + left_dur;
                        }
                        _ => {}
                    }
                    left.duration = left_dur;
                    layer.items.insert(pos + 1, right);
                    return Ok(());
                }
            }
            Err(CommandError::ItemNotFound(*item_id))
        }
        EditCommand::SetTransform { item_id, transform } => {
            let item = find_item_mut(project, *item_id)?;
            item.transform = *transform;
            Ok(())
        }
        EditCommand::SetBlendMode {
            item_id,
            blend_mode,
        } => {
            let item = find_item_mut(project, *item_id)?;
            item.blend_mode = *blend_mode;
            Ok(())
        }
        EditCommand::SetTextContent { item_id, content } => {
            let item = find_item_mut(project, *item_id)?;
            if let ItemKind::Text {
                content: existing, ..
            } = &mut item.kind
            {
                *existing = content.clone();
            }
            Ok(())
        }
        EditCommand::SetTextAnimator { item_id, animator } => {
            let item = find_item_mut(project, *item_id)?;
            if let ItemKind::Text {
                animator: existing, ..
            } = &mut item.kind
            {
                *existing = *animator;
            }
            Ok(())
        }
        EditCommand::SetKeyframe {
            item_id,
            property,
            keyframe,
        } => {
            let item = find_item_mut(project, *item_id)?;
            let track = item.keyframes.entry(property.clone()).or_default();
            if let Some(existing) = track.iter_mut().find(|k| k.time == keyframe.time) {
                *existing = *keyframe;
            } else {
                track.push(*keyframe);
                track.sort_by_key(|k| k.time);
            }
            Ok(())
        }
        EditCommand::RemoveKeyframe {
            item_id,
            property,
            time,
        } => {
            let item = find_item_mut(project, *item_id)?;
            if let Some(track) = item.keyframes.get_mut(property) {
                track.retain(|k| k.time != *time);
            }
            Ok(())
        }
        EditCommand::SetTrackMatte { layer_index, matte } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            let layer = scene
                .layers
                .get_mut(*layer_index)
                .ok_or(CommandError::InvalidLayer(*layer_index))?;
            layer.track_matte = *matte;
            Ok(())
        }
        EditCommand::PrecomposeSelection {
            item_ids,
            new_scene_id,
            precomp_name,
            precomp_item_id,
            target_layer_index,
        } => {
            if item_ids.is_empty() {
                return Err(CommandError::EmptyPrecomposeSelection);
            }
            let (width, height, fps) = {
                let active = project.active_scene().ok_or(CommandError::NoActiveScene)?;
                if *target_layer_index >= active.layers.len() {
                    return Err(CommandError::InvalidLayer(*target_layer_index));
                }
                (active.width, active.height, active.fps)
            };

            // 選択アイテムを元シーンからレイヤー順を保って抽出
            let mut extracted: Vec<(usize, TimelineItem)> = Vec::new();
            {
                let active = project
                    .active_scene_mut()
                    .ok_or(CommandError::NoActiveScene)?;
                for (layer_idx, layer) in active.layers.iter_mut().enumerate() {
                    let mut remaining = Vec::with_capacity(layer.items.len());
                    for item in layer.items.drain(..) {
                        if item_ids.contains(&item.id) {
                            extracted.push((layer_idx, item));
                        } else {
                            remaining.push(item);
                        }
                    }
                    layer.items = remaining;
                }
            }
            if extracted.is_empty() {
                return Err(CommandError::EmptyPrecomposeSelection);
            }

            let min_start = extracted
                .iter()
                .map(|(_, it)| it.start)
                .min()
                .unwrap_or(RationalTime::ZERO);
            let max_end = extracted
                .iter()
                .map(|(_, it)| it.end())
                .max()
                .unwrap_or(min_start + RationalTime::from_seconds(1.0));
            let precomp_duration = (max_end - min_start).max(RationalTime::from_ticks(1));

            let max_layer_idx = extracted.iter().map(|(l, _)| *l).max().unwrap_or(0);
            let mut new_layers: Vec<Layer> = (0..=max_layer_idx.max(2))
                .map(|i| {
                    let mut l = Layer::new(format!("Layer {}", i + 1));
                    // クラッシュ復旧リプレイ時の完全一致＆Precompハッシュキャッシュ整合性のため決定論的IDを割当
                    l.id =
                        uuid::Uuid::from_u128(new_scene_id.as_u128().wrapping_add((i + 1) as u128));
                    l
                })
                .collect();
            for (layer_idx, mut item) in extracted {
                item.start = item.start - min_start;
                new_layers[layer_idx].items.push(item);
            }

            let new_scene = Scene {
                id: *new_scene_id,
                name: precomp_name.clone(),
                width,
                height,
                fps,
                camera: Camera3D::default(),
                master_audio_effects: Vec::new(),
                target_lufs: None,
                audio_ducking: AudioDuckingConfig::default(),
                color_grade: SceneColorGrade::default(),
                color_management: crate::color_mgmt::ColorManagementConfig::default(),
                cinema_finishing: crate::finishing::CinemaFinishingConfig::default(),
                motion_blur: MotionBlurConfig::default(),
                markers: Vec::new(),
                node_graph: NodeCompositeGraph::default(),
                review_comments: Vec::new(),
                ab_compare: crate::studio_hub::AbCompareConfig::default(),
                optical_vfx: crate::v80_vfx::OpticalVfxConfig::default(),
                adr_session: crate::v80_vfx::AdrSessionConfig::default(),
                spatial_bus: crate::v80_vfx::SpatialBusConfig::default(),
                live_switcher: crate::live_switcher::LiveSwitcherConfig::default(),
                spatial_video: crate::spatial_video::SpatialVideoConfig::default(),
                lighting_3d_adv: crate::fluid_cloth::Lighting3DAdvConfig::default(),
                gpu_graph: crate::perf_engine::GpuRenderGraphConfig::default(),
                layers: new_layers,
            };
            project.scenes.push(new_scene);

            let mut precomp_clip = TimelineItem::new(
                precomp_name.clone(),
                min_start,
                precomp_duration,
                ItemKind::Precomp {
                    scene_id: *new_scene_id,
                },
            );
            precomp_clip.id = *precomp_item_id;

            let active = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            active.layers[*target_layer_index].items.push(precomp_clip);
            Ok(())
        }
        EditCommand::SwitchScene { scene_id } => {
            if project.scene(*scene_id).is_none() {
                return Err(CommandError::SceneNotFound(*scene_id));
            }
            project.active_scene_id = *scene_id;
            Ok(())
        }
        EditCommand::SetCamera3D { camera } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            scene.camera = *camera;
            Ok(())
        }
        EditCommand::SetTransform3D {
            item_id,
            transform_3d,
        } => {
            let item = find_item_mut(project, *item_id)?;
            item.transform_3d = *transform_3d;
            Ok(())
        }
        EditCommand::SetCharacterSprite { item_id, config } => {
            let item = find_item_mut(project, *item_id)?;
            if let ItemKind::CharacterSprite {
                config: existing, ..
            } = &mut item.kind
            {
                *existing = config.clone();
            }
            Ok(())
        }
        EditCommand::ImportSubtitles { layer_index, items } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            let layer = scene
                .layers
                .get_mut(*layer_index)
                .ok_or(CommandError::InvalidLayer(*layer_index))?;
            layer.items.extend(items.iter().cloned());
            Ok(())
        }
        EditCommand::AddEffect { item_id, effect } => {
            let item = find_item_mut(project, *item_id)?;
            item.effect_chain.push(effect.clone());
            Ok(())
        }
        EditCommand::UpdateEffectParams {
            item_id,
            effect_id,
            params,
        } => {
            let item = find_item_mut(project, *item_id)?;
            let eff = item
                .effect_chain
                .iter_mut()
                .find(|e| e.id == *effect_id)
                .ok_or(CommandError::EffectNotFound(*effect_id))?;
            for (k, v) in params {
                eff.params.insert(k.clone(), *v);
            }
            Ok(())
        }
        EditCommand::RemoveEffect { item_id, effect_id } => {
            let item = find_item_mut(project, *item_id)?;
            let prev = item.effect_chain.len();
            item.effect_chain.retain(|e| e.id != *effect_id);
            if item.effect_chain.len() == prev {
                return Err(CommandError::EffectNotFound(*effect_id));
            }
            Ok(())
        }
        EditCommand::ToggleEffect {
            item_id,
            effect_id,
            enabled,
        } => {
            let item = find_item_mut(project, *item_id)?;
            let eff = item
                .effect_chain
                .iter_mut()
                .find(|e| e.id == *effect_id)
                .ok_or(CommandError::EffectNotFound(*effect_id))?;
            eff.enabled = *enabled;
            Ok(())
        }
        EditCommand::ReorderEffect {
            item_id,
            from_index,
            to_index,
        } => {
            let item = find_item_mut(project, *item_id)?;
            let len = item.effect_chain.len();
            if *from_index < len && *to_index < len && from_index != to_index {
                let eff = item.effect_chain.remove(*from_index);
                item.effect_chain.insert(*to_index, eff);
            }
            Ok(())
        }
        EditCommand::PasteEffects { item_id, effects } => {
            let item = find_item_mut(project, *item_id)?;
            item.effect_chain.extend(effects.iter().cloned());
            Ok(())
        }
        EditCommand::SetColorPalette { palette } => {
            project.color_palette = palette.clone();
            Ok(())
        }
        EditCommand::SetItemColor { item_id, color } => {
            let item = find_item_mut(project, *item_id)?;
            match &mut item.kind {
                ItemKind::Text {
                    color: existing, ..
                }
                | ItemKind::Shape {
                    color: existing, ..
                } => {
                    *existing = *color;
                }
                _ => {}
            }
            Ok(())
        }
        EditCommand::SetTextStyle { item_id, style } => {
            let item = find_item_mut(project, *item_id)?;
            item.text_style = *style;
            Ok(())
        }
        EditCommand::SetTextFont {
            item_id,
            font_family,
            font_size,
        } => {
            let item = find_item_mut(project, *item_id)?;
            if let ItemKind::Text {
                font_family: ff,
                font_size: fs,
                ..
            } = &mut item.kind
            {
                *ff = font_family.clone();
                *fs = font_size.clamp(6.0, 400.0);
            }
            Ok(())
        }
        EditCommand::SetFavoriteFonts { fonts } => {
            project.favorite_fonts = fonts.clone();
            Ok(())
        }
        EditCommand::SetLayerSettings {
            layer_index,
            name,
            visible,
            muted,
            offset_x,
            offset_y,
            volume,
            color_tag,
        } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            let layer = scene
                .layers
                .get_mut(*layer_index)
                .ok_or(CommandError::InvalidLayer(*layer_index))?;
            layer.name = name.clone();
            layer.visible = *visible;
            layer.muted = *muted;
            layer.offset_x = *offset_x;
            layer.offset_y = *offset_y;
            layer.volume = volume.clamp(0.0, 4.0);
            layer.color_tag = *color_tag;
            Ok(())
        }
        EditCommand::SetPlaybackSpeed {
            item_id,
            speed,
            loop_playback,
        } => {
            let item = find_item_mut(project, *item_id)?;
            item.playback.speed = speed.clamp(-8.0, 8.0);
            item.playback.loop_playback = *loop_playback;
            Ok(())
        }
        EditCommand::SetTransition {
            item_id,
            transition,
            softness,
        } => {
            let item = find_item_mut(project, *item_id)?;
            if let ItemKind::Transition {
                transition: t,
                softness: s,
                ..
            } = &mut item.kind
            {
                *t = *transition;
                *s = softness.clamp(0.0, 0.5);
            }
            Ok(())
        }
        EditCommand::AddAudioEffect { item_id, effect } => {
            if let Some(id) = item_id {
                let item = find_item_mut(project, *id)?;
                item.audio_effects.push(effect.clone());
            } else {
                let scene = project
                    .active_scene_mut()
                    .ok_or(CommandError::NoActiveScene)?;
                scene.master_audio_effects.push(effect.clone());
            }
            Ok(())
        }
        EditCommand::RemoveAudioEffect { item_id, effect_id } => {
            let chain = if let Some(id) = item_id {
                &mut find_item_mut(project, *id)?.audio_effects
            } else {
                &mut project
                    .active_scene_mut()
                    .ok_or(CommandError::NoActiveScene)?
                    .master_audio_effects
            };
            let prev = chain.len();
            chain.retain(|e| e.id != *effect_id);
            if chain.len() == prev {
                return Err(CommandError::EffectNotFound(*effect_id));
            }
            Ok(())
        }
        EditCommand::UpdateAudioEffect {
            item_id,
            effect_id,
            enabled,
            params,
        } => {
            let chain = if let Some(id) = item_id {
                &mut find_item_mut(project, *id)?.audio_effects
            } else {
                &mut project
                    .active_scene_mut()
                    .ok_or(CommandError::NoActiveScene)?
                    .master_audio_effects
            };
            let eff = chain
                .iter_mut()
                .find(|e| e.id == *effect_id)
                .ok_or(CommandError::EffectNotFound(*effect_id))?;
            eff.enabled = *enabled;
            for (k, v) in params {
                eff.params.insert(k.clone(), *v);
            }
            Ok(())
        }
        EditCommand::SetTargetLufs { target_lufs } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            scene.target_lufs = target_lufs.map(|v| v.clamp(-36.0, -3.0));
            Ok(())
        }
        EditCommand::AddMarker { marker } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            scene.markers.push(marker.clone());
            scene.markers.sort_by_key(|m| m.time);
            Ok(())
        }
        EditCommand::RemoveMarker { marker_id } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            scene.markers.retain(|m| m.id != *marker_id);
            Ok(())
        }
        EditCommand::SetSceneSettings {
            width,
            height,
            fps,
            rescale_items,
        } => {
            let new_w = (*width).clamp(16, 8192);
            let new_h = (*height).clamp(16, 8192);
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            let old_w = scene.width.max(1) as f32;
            let old_h = scene.height.max(1) as f32;
            scene.width = new_w;
            scene.height = new_h;
            scene.fps = *fps;

            if *rescale_items {
                let sx = new_w as f32 / old_w;
                let sy = new_h as f32 / old_h;
                let uniform = sx.min(sy);
                for layer in &mut scene.layers {
                    layer.offset_x *= sx;
                    layer.offset_y *= sy;
                    for item in &mut layer.items {
                        item.transform.x *= sx;
                        item.transform.y *= sy;
                        if let Some(kfs) = item.keyframes.get_mut("x") {
                            for kf in kfs {
                                kf.value *= sx;
                            }
                        }
                        if let Some(kfs) = item.keyframes.get_mut("y") {
                            for kf in kfs {
                                kf.value *= sy;
                            }
                        }
                        match &mut item.kind {
                            ItemKind::Shape { width, height, .. }
                            | ItemKind::CharacterSprite { width, height, .. }
                            | ItemKind::AudioSpectrum { width, height, .. } => {
                                *width = (*width * uniform).max(4.0);
                                *height = (*height * uniform).max(4.0);
                            }
                            ItemKind::Text { font_size, .. } => {
                                *font_size = (*font_size * uniform).clamp(8.0, 400.0);
                            }
                            _ => {}
                        }
                    }
                }
            }
            project.settings.width = new_w;
            project.settings.height = new_h;
            project.settings.fps = *fps;
            Ok(())
        }
        EditCommand::SetAudioDucking { ducking } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            scene.audio_ducking = *ducking;
            Ok(())
        }
        EditCommand::SetAudioReactive { item_id, reactive } => {
            let item = find_item_mut(project, *item_id)?;
            item.audio_reactive = *reactive;
            Ok(())
        }
        EditCommand::SetMasks { item_id, masks } => {
            let item = find_item_mut(project, *item_id)?;
            item.masks = masks.clone();
            Ok(())
        }
        EditCommand::CreateScene { scene, switch_to } => {
            let new_id = scene.id;
            if let Some(existing) = project.scenes.iter_mut().find(|s| s.id == new_id) {
                *existing = scene.clone();
            } else {
                project.scenes.push(scene.clone());
            }
            if *switch_to {
                project.active_scene_id = new_id;
            }
            Ok(())
        }
        EditCommand::SetFilterObject {
            item_id,
            effect_type,
            scope,
            params,
        } => {
            let item = find_item_mut(project, *item_id)?;
            if let ItemKind::FilterObject {
                effect_type: et,
                scope: sc,
                params: pm,
            } = &mut item.kind
            {
                *et = effect_type.clone();
                *sc = *scope;
                *pm = params.clone();
            }
            Ok(())
        }
        EditCommand::SetGroupControl {
            item_id,
            target_layer_count,
        } => {
            let item = find_item_mut(project, *item_id)?;
            if let ItemKind::GroupControl {
                target_layer_count: cnt,
            } = &mut item.kind
            {
                *cnt = (*target_layer_count).clamp(1, 64);
            }
            Ok(())
        }
        EditCommand::SetAudioSpectrum {
            item_id,
            linked_audio_layer,
            width,
            height,
            bar_count,
            color,
            peak_color,
            sensitivity,
        } => {
            let item = find_item_mut(project, *item_id)?;
            if let ItemKind::AudioSpectrum {
                linked_audio_layer: l,
                width: w,
                height: h,
                bar_count: bc,
                color: c,
                peak_color: pc,
                sensitivity: sens,
            } = &mut item.kind
            {
                *l = *linked_audio_layer;
                *w = width.clamp(16.0, 8192.0);
                *h = height.clamp(16.0, 8192.0);
                *bc = (*bar_count).clamp(4, 64);
                *c = *color;
                *pc = *peak_color;
                *sens = sensitivity.clamp(0.1, 10.0);
            }
            Ok(())
        }
        EditCommand::UpdateMarker {
            marker_id,
            label,
            color,
        } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            if let Some(m) = scene.markers.iter_mut().find(|m| m.id == *marker_id) {
                m.label = label.clone();
                m.color = *color;
            }
            Ok(())
        }
        EditCommand::SetSceneColorGrade { grade } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            let mut g = grade.clone();
            g.exposure = g.exposure.clamp(-3.0, 3.0);
            g.contrast = g.contrast.clamp(0.2, 2.5);
            g.saturation = g.saturation.clamp(0.0, 2.5);
            g.temperature = g.temperature.clamp(-1.0, 1.0);
            g.tint = g.tint.clamp(-1.0, 1.0);
            g.lut_intensity = g.lut_intensity.clamp(0.0, 1.0);
            for ch in &mut g.lift {
                *ch = ch.clamp(-0.5, 0.5);
            }
            for ch in &mut g.gamma {
                *ch = ch.clamp(-0.5, 0.5);
            }
            for ch in &mut g.gain {
                *ch = ch.clamp(0.2, 2.5);
            }
            scene.color_grade = g;
            Ok(())
        }
        EditCommand::SetExpression {
            item_id,
            property,
            expression,
        } => {
            let item = find_item_mut(project, *item_id)?;
            match expression.as_deref().map(str::trim) {
                None | Some("") => {
                    item.expressions.remove(property);
                }
                Some(src) => {
                    compile_expression(src)
                        .map_err(|e| CommandError::InvalidExpression(e.to_string()))?;
                    item.expressions.insert(property.clone(), src.to_string());
                }
            }
            Ok(())
        }
        EditCommand::UpdateTemplateInstance {
            item_id,
            values,
            duration_sec,
        } => {
            let (precomp_scene_id, binding_snap, old_dur, new_dur) = {
                let item = find_item_mut(project, *item_id)?;
                let ItemKind::Precomp { scene_id } = item.kind else {
                    return Ok(());
                };
                let Some(binding) = item.template_binding.as_mut() else {
                    return Ok(());
                };
                for (k, v) in values {
                    binding.values.insert(k.clone(), v.clone());
                }
                let prev_dur = binding.current_duration_sec.max(0.1);
                let target_dur = duration_sec
                    .map(|d| d.clamp(0.3, 600.0))
                    .unwrap_or(prev_dur);
                if (target_dur - prev_dur).abs() > 1e-4 {
                    binding.current_duration_sec = target_dur;
                    item.duration = RationalTime::from_seconds(target_dur);
                }
                (scene_id, binding.clone(), prev_dur, target_dur)
            };

            if let Some(nested) = project.scenes.iter_mut().find(|s| s.id == precomp_scene_id) {
                apply_template_values_to_scene(
                    nested,
                    &binding_snap.exposed_params,
                    &binding_snap.values,
                );
                if (new_dur - old_dur).abs() > 1e-4 {
                    apply_responsive_time_stretch(
                        nested,
                        old_dur,
                        new_dur,
                        binding_snap.responsive_time.intro_sec,
                        binding_snap.responsive_time.outro_sec,
                    );
                }
            }
            Ok(())
        }
        EditCommand::SetParticleConfig { item_id, config } => {
            let item = find_item_mut(project, *item_id)?;
            if let ItemKind::ParticleEmitter { config: dst } = &mut item.kind {
                let mut c = *config;
                c.count = c.count.clamp(1, 600);
                c.speed = c.speed.clamp(0.0, 4000.0);
                c.size_px = c.size_px.clamp(1.0, 200.0);
                c.spread_deg = c.spread_deg.clamp(0.0, 360.0);
                c.lifetime_sec = c.lifetime_sec.clamp(0.1, 30.0);
                c.depth_3d = c.depth_3d.clamp(0.0, 1.0);
                *dst = c;
            }
            Ok(())
        }
        EditCommand::SetVectorShape { item_id, config } => {
            let item = find_item_mut(project, *item_id)?;
            if let ItemKind::VectorShape { config: dst } = &mut item.kind {
                let mut c = config.clone();
                c.width = c.width.clamp(8.0, 8192.0);
                c.height = c.height.clamp(8.0, 8192.0);
                c.stroke_width = c.stroke_width.clamp(0.0, 128.0);
                c.dash_length = c.dash_length.clamp(0.0, 500.0);
                c.dash_gap = c.dash_gap.clamp(0.0, 500.0);
                c.trim_start = c.trim_start.clamp(0.0, 1.0);
                c.trim_end = c.trim_end.clamp(0.0, 1.0);
                *dst = c;
            }
            Ok(())
        }
        EditCommand::SetMotionBlur { motion_blur } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            let mut mb = *motion_blur;
            mb.shutter_angle_deg = mb.shutter_angle_deg.clamp(0.0, 360.0);
            mb.shutter_phase_deg = mb.shutter_phase_deg.clamp(-180.0, 180.0);
            mb.preview_samples = mb.preview_samples.clamp(1, 8);
            mb.export_samples = mb.export_samples.clamp(1, 16);
            scene.motion_blur = mb;
            Ok(())
        }
        EditCommand::SetGroupId { item_ids, group_id } => {
            for &id in item_ids {
                if let Ok(item) = find_item_mut(project, id) {
                    item.group_id = *group_id;
                }
            }
            Ok(())
        }
        EditCommand::SlipTrim { item_id, delta } => {
            let item = find_item_mut(project, *item_id)?;
            match &mut item.kind {
                ItemKind::Video { source_offset, .. } | ItemKind::Audio { source_offset, .. } => {
                    *source_offset = (*source_offset + *delta).max(RationalTime::ZERO);
                }
                _ => {}
            }
            Ok(())
        }
        EditCommand::RollTrim {
            left_item_id,
            right_item_id,
            delta,
        } => {
            let min_dur = RationalTime::from_seconds(0.05);
            let (left_dur, right_start, right_dur) = {
                let left = find_item_ref(project, *left_item_id)?;
                let right = find_item_ref(project, *right_item_id)?;
                (left.duration, right.start, right.duration)
            };
            let max_pos = (right_dur - min_dur).max(RationalTime::ZERO);
            let max_neg = (min_dur - left_dur).min(RationalTime::ZERO);
            let clamped = (*delta).clamp(max_neg, max_pos);

            let left = find_item_mut(project, *left_item_id)?;
            left.duration = (left.duration + clamped).max(min_dur);

            let right = find_item_mut(project, *right_item_id)?;
            right.start = (right_start + clamped).max(RationalTime::ZERO);
            right.duration = (right_dur - clamped).max(min_dur);
            match &mut right.kind {
                ItemKind::Video { source_offset, .. } | ItemKind::Audio { source_offset, .. } => {
                    *source_offset = (*source_offset + clamped).max(RationalTime::ZERO);
                }
                _ => {}
            }
            Ok(())
        }
        EditCommand::SlideTrim {
            item_id,
            left_neighbor_id,
            right_neighbor_id,
            delta,
        } => {
            let min_dur = RationalTime::from_seconds(0.05);
            let item_start = find_item_ref(project, *item_id)?.start;
            let mut clamped = *delta;
            if item_start + clamped < RationalTime::ZERO {
                clamped = RationalTime::ZERO - item_start;
            }
            if let Some(lid) = left_neighbor_id
                && let Ok(l) = find_item_ref(project, *lid)
            {
                let min_delta = min_dur - l.duration;
                if clamped < min_delta {
                    clamped = min_delta;
                }
            }
            if let Some(rid) = right_neighbor_id
                && let Ok(r) = find_item_ref(project, *rid)
            {
                let max_delta = r.duration - min_dur;
                if clamped > max_delta {
                    clamped = max_delta;
                }
            }

            let mid = find_item_mut(project, *item_id)?;
            mid.start = (mid.start + clamped).max(RationalTime::ZERO);

            if let Some(lid) = left_neighbor_id
                && let Ok(l) = find_item_mut(project, *lid)
            {
                l.duration = (l.duration + clamped).max(min_dur);
            }
            if let Some(rid) = right_neighbor_id
                && let Ok(r) = find_item_mut(project, *rid)
            {
                r.start = (r.start + clamped).max(RationalTime::ZERO);
                r.duration = (r.duration - clamped).max(min_dur);
                match &mut r.kind {
                    ItemKind::Video { source_offset, .. }
                    | ItemKind::Audio { source_offset, .. } => {
                        *source_offset = (*source_offset + clamped).max(RationalTime::ZERO);
                    }
                    _ => {}
                }
            }
            Ok(())
        }
        EditCommand::SetLayerMixer {
            layer_index,
            volume,
            pan,
            muted,
            solo,
            bus,
            send_reverb,
        } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            let layer = scene
                .layers
                .get_mut(*layer_index)
                .ok_or(CommandError::InvalidLayer(*layer_index))?;
            layer.volume = volume.clamp(0.0, 4.0);
            layer.pan = pan.clamp(-1.0, 1.0);
            layer.muted = *muted;
            layer.solo = *solo;
            layer.bus = if bus.trim().is_empty() {
                "Master".to_string()
            } else {
                bus.trim().to_string()
            };
            layer.send_reverb = send_reverb.clamp(0.0, 1.0);
            Ok(())
        }
        EditCommand::SetNodeGraph { graph } => {
            if let Err(err) = graph.topological_order() {
                return Err(CommandError::InvalidExpression(err.to_string()));
            }
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            scene.node_graph = graph.clone();
            Ok(())
        }
        EditCommand::SetOpticalFlowInterp { item_id, enabled } => {
            let item = find_item_mut(project, *item_id)?;
            item.playback.optical_flow_interp = *enabled;
            Ok(())
        }
        EditCommand::SetReviewComments { comments } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            let mut sorted = comments.clone();
            sorted.sort_by_key(|c| c.time);
            scene.review_comments = sorted;
            Ok(())
        }
        EditCommand::ReplaceProjectSnapshot {
            project: next_project,
        } => {
            *project = (**next_project).clone();
            Ok(())
        }
        EditCommand::SetCornerPin {
            item_id,
            corner_pin,
        } => {
            let item = find_item_mut(project, *item_id)?;
            item.corner_pin = *corner_pin;
            Ok(())
        }
        EditCommand::SetKineticSubtitle { item_id, kinetic } => {
            let item = find_item_mut(project, *item_id)?;
            item.kinetic_subtitle = kinetic.clone();
            Ok(())
        }
        EditCommand::SetMesh3D { item_id, config } => {
            let item = find_item_mut(project, *item_id)?;
            if let ItemKind::Mesh3D { config: ref mut c } = item.kind {
                *c = config.clone();
                Ok(())
            } else {
                Err(CommandError::ItemNotFound(*item_id))
            }
        }
        EditCommand::SetItemPhonemes { item_id, phonemes } => {
            let item = find_item_mut(project, *item_id)?;
            item.phoneme_segments = phonemes.clone();
            Ok(())
        }
        EditCommand::SetColorManagement { config } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            let mut c = *config;
            c.exposure_stops = c.exposure_stops.clamp(-4.0, 4.0);
            scene.color_management = c;
            Ok(())
        }
        EditCommand::SetGenerativeBg { item_id, config } => {
            let item = find_item_mut(project, *item_id)?;
            if let ItemKind::GenerativeBg { config: ref mut c } = item.kind {
                let mut next = *config;
                next.speed = next.speed.clamp(0.0, 10.0);
                next.scale = next.scale.clamp(0.1, 10.0);
                next.complexity = next.complexity.clamp(1, 8);
                next.width = next.width.clamp(32.0, 8192.0);
                next.height = next.height.clamp(32.0, 8192.0);
                *c = next;
                Ok(())
            } else {
                Err(CommandError::ItemNotFound(*item_id))
            }
        }
        EditCommand::SetRig2D { item_id, config } => {
            let item = find_item_mut(project, *item_id)?;
            if let ItemKind::RiggedPuppet { config: ref mut c } = item.kind {
                let mut next = (**config).clone();
                next.width = next.width.clamp(64.0, 4096.0);
                next.height = next.height.clamp(64.0, 4096.0);
                next.motion_speed = next.motion_speed.clamp(0.0, 6.0);
                next.motion_amp = next.motion_amp.clamp(0.0, 3.0);
                **c = next;
                Ok(())
            } else {
                Err(CommandError::ItemNotFound(*item_id))
            }
        }
        EditCommand::SetSpeedRamp {
            item_id,
            speed_ramp,
        } => {
            let item = find_item_mut(project, *item_id)?;
            let mut sr = speed_ramp.clone();
            sr.knots = sr.normalized();
            item.speed_ramp = sr;
            Ok(())
        }
        EditCommand::SetCinemaFinishing { config } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            let mut next = config.clone();
            next.halation_strength = next.halation_strength.clamp(0.0, 1.5);
            next.halation_threshold = next.halation_threshold.clamp(0.3, 1.0);
            next.anamorphic_flare = next.anamorphic_flare.clamp(0.0, 1.5);
            next.film_grain = next.film_grain.clamp(0.0, 0.5);
            next.vignette = next.vignette.clamp(0.0, 1.0);
            scene.cinema_finishing = next;
            Ok(())
        }
        EditCommand::SetDataChart { item_id, config } => {
            let item = find_item_mut(project, *item_id)?;
            if let ItemKind::DataChart { config: ref mut c } = item.kind {
                let mut next = (**config).clone();
                next.width = next.width.clamp(120.0, 4096.0);
                next.height = next.height.clamp(80.0, 4096.0);
                next.animate_in_sec = next.animate_in_sec.clamp(0.05, 30.0);
                **c = next;
                Ok(())
            } else {
                Err(CommandError::ItemNotFound(*item_id))
            }
        }
        EditCommand::SetVfxPaint { item_id, vfx_paint } => {
            let item = find_item_mut(project, *item_id)?;
            let mut next = vfx_paint.clone();
            for stroke in &mut next.strokes {
                stroke.radius_px = stroke.radius_px.clamp(1.0, 256.0);
                stroke.hardness = stroke.hardness.clamp(0.05, 1.0);
                stroke.opacity = stroke.opacity.clamp(0.0, 1.0);
                stroke.write_on_progress = stroke.write_on_progress.clamp(0.0, 1.0);
            }
            item.vfx_paint = next;
            Ok(())
        }
        EditCommand::SetAssetConformMeta { asset_id, meta } => {
            project.conform_meta.insert(*asset_id, meta.clone());
            Ok(())
        }
        EditCommand::ConformClipSource {
            item_id,
            new_asset_id,
            new_source_offset,
        } => {
            let item = find_item_mut(project, *item_id)?;
            let clamped_off = (*new_source_offset).max(RationalTime::ZERO);
            match &mut item.kind {
                ItemKind::Video {
                    asset_id,
                    source_offset,
                    ..
                }
                | ItemKind::Audio {
                    asset_id,
                    source_offset,
                    ..
                } => {
                    *asset_id = *new_asset_id;
                    *source_offset = clamped_off;
                    Ok(())
                }
                ItemKind::Image { asset_id } => {
                    *asset_id = *new_asset_id;
                    Ok(())
                }
                _ => Err(CommandError::ItemNotFound(*item_id)),
            }
        }
        EditCommand::SetAbCompare { ab_compare } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            let mut next = ab_compare.clone();
            next.split_ratio = next.split_ratio.clamp(0.05, 0.95);
            next.diff_gain = next.diff_gain.clamp(1.0, 16.0);
            next.reference_sec = next.reference_sec.max(0.0);
            scene.ab_compare = next;
            Ok(())
        }
        EditCommand::SetRotoBrush {
            item_id,
            roto_brush,
        } => {
            let item = find_item_mut(project, *item_id)?;
            let mut next = roto_brush.clone();
            next.edge_snap_radius_px = next.edge_snap_radius_px.clamp(1, 48);
            next.edge_weight = next.edge_weight.clamp(0.05, 1.0);
            next.keyframes.sort_by_key(|k| k.time);
            item.roto_brush = next;
            Ok(())
        }
        EditCommand::SetSvgMorph { item_id, config } => {
            let item = find_item_mut(project, *item_id)?;
            if let ItemKind::SvgMorph { config: ref mut c } = item.kind {
                let mut next = (**config).clone();
                next.width = next.width.clamp(32.0, 4096.0);
                next.height = next.height.clamp(32.0, 4096.0);
                next.morph_progress = next.morph_progress.clamp(0.0, 1.0);
                next.auto_morph_hz = next.auto_morph_hz.clamp(0.0, 8.0);
                next.stroke_width = next.stroke_width.clamp(0.0, 64.0);
                next.trim_start = next.trim_start.clamp(0.0, 1.0);
                next.trim_end = next.trim_end.clamp(0.0, 1.0);
                next.sample_count = next.sample_count.clamp(12, 256);
                **c = next;
                Ok(())
            } else {
                Err(CommandError::ItemNotFound(*item_id))
            }
        }
        EditCommand::SetTextOnPath {
            item_id,
            text_on_path,
        } => {
            let item = find_item_mut(project, *item_id)?;
            let mut next = text_on_path.clone();
            next.path_offset = next.path_offset.clamp(-1.0, 2.0);
            next.animate_speed = next.animate_speed.clamp(-2.0, 2.0);
            next.letter_spacing_px = next.letter_spacing_px.clamp(-24.0, 96.0);
            next.vertical_offset_px = next.vertical_offset_px.clamp(-200.0, 200.0);
            item.text_on_path = next;
            Ok(())
        }
        EditCommand::SetWarpDisplace {
            item_id,
            warp_displace,
        } => {
            let item = find_item_mut(project, *item_id)?;
            let mut next = warp_displace.clone();
            next.amount = next.amount.clamp(-1.0, 1.0);
            next.frequency = next.frequency.clamp(0.2, 16.0);
            next.speed = next.speed.clamp(-10.0, 10.0);
            next.luma_displace_px = next.luma_displace_px.clamp(-64.0, 64.0);
            next.ensure_grid_size();
            item.warp_displace = next;
            Ok(())
        }
        EditCommand::SetOpticalVfx { optical_vfx } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            let mut next = optical_vfx.clone();
            next.focus_depth = next.focus_depth.clamp(0.0, 1.0);
            next.focus_band = next.focus_band.clamp(0.02, 0.6);
            next.bokeh_radius_px = next.bokeh_radius_px.clamp(0.0, 20.0);
            next.bokeh_highlight_boost = next.bokeh_highlight_boost.clamp(0.0, 3.0);
            next.bokeh_blades = if next.bokeh_blades == 0 {
                0
            } else {
                next.bokeh_blades.clamp(5, 8)
            };
            next.light_pos = [
                next.light_pos[0].clamp(-0.5, 1.5),
                next.light_pos[1].clamp(-0.5, 1.5),
            ];
            next.god_rays_intensity = next.god_rays_intensity.clamp(0.0, 2.0);
            next.god_rays_decay = next.god_rays_decay.clamp(0.80, 0.99);
            next.god_rays_threshold = next.god_rays_threshold.clamp(0.1, 2.0);
            next.god_rays_samples = next.god_rays_samples.clamp(8, 64);
            next.k1 = next.k1.clamp(-0.6, 0.6);
            next.k2 = next.k2.clamp(-0.4, 0.4);
            next.p1 = next.p1.clamp(-0.15, 0.15);
            next.p2 = next.p2.clamp(-0.15, 0.15);
            next.chromatic_aberration = next.chromatic_aberration.clamp(-0.05, 0.05);
            scene.optical_vfx = next;
            Ok(())
        }
        EditCommand::SetAdrSession { adr_session } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            let mut next = adr_session.clone();
            next.cues.sort_by_key(|c| c.start_time);
            if !next.cues.is_empty() {
                next.active_cue_idx = next.active_cue_idx.min(next.cues.len() - 1);
            } else {
                next.active_cue_idx = 0;
            }
            scene.adr_session = next;
            Ok(())
        }
        EditCommand::SetSpatialBus { spatial_bus } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            let mut next = spatial_bus.clone();
            next.azimuth_deg = next.azimuth_deg.clamp(-180.0, 180.0);
            next.elevation_deg = next.elevation_deg.clamp(-90.0, 90.0);
            next.lfe_crossover_hz = next.lfe_crossover_hz.clamp(40.0, 160.0);
            next.center_dialogue_boost = next.center_dialogue_boost.clamp(0.0, 1.0);
            next.room_width = next.room_width.clamp(0.0, 1.0);
            next.head_yaw_deg = next.head_yaw_deg.clamp(-180.0, 180.0);
            next.head_pitch_deg = next.head_pitch_deg.clamp(-90.0, 90.0);
            scene.spatial_bus = next;
            Ok(())
        }
        EditCommand::SetLiveSwitcher { live_switcher } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            let mut next = live_switcher.clone();
            next.transition_progress = next.transition_progress.clamp(0.0, 1.0);
            if !next.sources.is_empty() {
                let max_i = next.sources.len() - 1;
                next.program_channel = next.program_channel.min(max_i);
                next.preview_channel = next.preview_channel.min(max_i);
            }
            scene.live_switcher = next;
            Ok(())
        }
        EditCommand::SetVtuberAvatar { item_id, config } => {
            let item = find_item_mut(project, *item_id)?;
            let mut next = (**config).clone();
            next.width = next.width.clamp(64.0, 2048.0);
            next.height = next.height.clamp(64.0, 2048.0);
            next.grid_cols = next.grid_cols.clamp(3, 18);
            next.grid_rows = next.grid_rows.clamp(3, 18);
            item.kind = ItemKind::VtuberAvatar {
                config: Box::new(next),
            };
            Ok(())
        }
        EditCommand::SetHardwareController { hardware_ctrl } => {
            project.hardware_ctrl = hardware_ctrl.clone();
            Ok(())
        }
        EditCommand::SetSmartDamIndex { smart_dam } => {
            project.smart_dam = smart_dam.clone();
            Ok(())
        }
        EditCommand::RelinkAssets { relinks } => {
            for (aid, new_path) in relinks {
                if let Some(asset) = project.assets.iter_mut().find(|a| a.id == *aid) {
                    asset.fallback_absolute_path = Some(std::path::PathBuf::from(new_path));
                    asset.relative_path = None;
                }
                if let Some(rec) = project
                    .smart_dam
                    .records
                    .iter_mut()
                    .find(|r| r.asset_id == Some(*aid))
                {
                    rec.file_path = new_path.clone();
                }
            }
            Ok(())
        }
        EditCommand::SetSpatialVideo { spatial_video } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            let mut next = spatial_video.clone();
            next.yaw_deg = next.yaw_deg.clamp(-180.0, 180.0);
            next.pitch_deg = next.pitch_deg.clamp(-90.0, 90.0);
            next.roll_deg = next.roll_deg.clamp(-180.0, 180.0);
            next.fov_deg = next.fov_deg.clamp(25.0, 155.0);
            next.tiny_planet_zoom = next.tiny_planet_zoom.clamp(0.25, 4.0);
            next.horizon_lock_smoothing = next.horizon_lock_smoothing.clamp(0.0, 0.98);
            next.ipd_parallax_px = next.ipd_parallax_px.clamp(0.0, 64.0);
            next.convergence_offset_px = next.convergence_offset_px.clamp(-48.0, 48.0);
            scene.spatial_video = next;
            Ok(())
        }
        EditCommand::SetLighting3DAdv { lighting_3d_adv } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            let mut next = lighting_3d_adv.clone();
            next.shadow_map_size = next.shadow_map_size.clamp(64, 512);
            next.shadow_bias = next.shadow_bias.clamp(0.001, 0.25);
            next.shadow_softness_pcf = next.shadow_softness_pcf.clamp(1, 3);
            next.shadow_strength = next.shadow_strength.clamp(0.0, 1.0);
            next.hdri_rotation_deg = next.hdri_rotation_deg.rem_euclid(360.0);
            next.hdri_exposure = next.hdri_exposure.clamp(0.05, 5.0);
            scene.lighting_3d_adv = next;
            Ok(())
        }
        EditCommand::SetFluidSim { item_id, config } => {
            let item = find_item_mut(project, *item_id)?;
            let mut next = (**config).clone();
            next.grid_w = next.grid_w.clamp(16, 96);
            next.grid_h = next.grid_h.clamp(16, 96);
            next.width = next.width.clamp(64.0, 2048.0);
            next.height = next.height.clamp(64.0, 2048.0);
            item.kind = ItemKind::FluidSim {
                config: Box::new(next),
            };
            Ok(())
        }
        EditCommand::SetClothSim { item_id, config } => {
            let item = find_item_mut(project, *item_id)?;
            let mut next = (**config).clone();
            next.cols = next.cols.clamp(4, 28);
            next.rows = next.rows.clamp(4, 28);
            next.width = next.width.clamp(64.0, 2048.0);
            next.height = next.height.clamp(64.0, 2048.0);
            item.kind = ItemKind::ClothSim {
                config: Box::new(next),
            };
            Ok(())
        }
        EditCommand::SetRenderFarm { render_farm } => {
            project.render_farm = render_farm.clone();
            Ok(())
        }
        EditCommand::SetGpuRenderGraph { gpu_graph } => {
            let scene = project
                .active_scene_mut()
                .ok_or(CommandError::NoActiveScene)?;
            let mut next = gpu_graph.clone();
            next.max_vram_textures_mib = next.max_vram_textures_mib.clamp(64, 16_384);
            scene.gpu_graph = next;
            Ok(())
        }
        EditCommand::SetAdaptivePlayback { adaptive_playback } => {
            let mut next = adaptive_playback.clone();
            next.target_fps = next.target_fps.clamp(15.0, 240.0);
            next.min_scale_divisor = next.min_scale_divisor.clamp(1, 4);
            next.max_scale_divisor = next.max_scale_divisor.clamp(next.min_scale_divisor, 4);
            next.drop_threshold_ratio = next.drop_threshold_ratio.clamp(1.0, 3.0);
            next.recover_threshold_ratio = next.recover_threshold_ratio.clamp(0.2, 0.95);
            next.prerender_budget_frames = next.prerender_budget_frames.clamp(1, 2000);
            project.adaptive_playback = next;
            Ok(())
        }
        EditCommand::SetProfilerHud { profiler_hud } => {
            let mut next = profiler_hud.clone();
            next.warn_frame_ms = next.warn_frame_ms.clamp(4.0, 200.0);
            project.profiler_hud = next;
            Ok(())
        }
        EditCommand::Batch { commands, .. } => {
            for cmd in commands {
                apply_command(project, cmd)?;
            }
            Ok(())
        }
    }
}
