//! `CommandBus` ＆ `EditCommand` 統合単体テスト

use super::*;
use crate::animation::{CubicBezier, Interpolation};
use crate::project::{ProjectSettings, RgbaColor, ShapeKind};
use tempfile::tempdir;
use uuid::Uuid;

fn sample_item(name: &str) -> TimelineItem {
    TimelineItem::new(
        name,
        RationalTime::ZERO,
        RationalTime::from_seconds(2.0),
        ItemKind::Shape {
            shape: ShapeKind::Rectangle,
            width: 100.0,
            height: 100.0,
            color: RgbaColor::WHITE,
        },
    )
}

#[test]
fn undo_redo_coalescing_and_gui_drag_lock() {
    let mut project = Project::new("BusTest", ProjectSettings::default());
    let mut bus = CommandBus::new();
    let item = sample_item("Clip1");
    let id = item.id;

    bus.execute(
        &mut project,
        EditCommand::AddObject {
            layer_index: 0,
            item,
        },
        CommandSource::Gui,
        None,
    )
    .unwrap();

    // 連続ドラッグ移動を同じ coalesce_key で2回実行 -> Undo単位は1つだけ増える
    for sec in [1.0, 1.5, 2.0] {
        bus.execute(
            &mut project,
            EditCommand::MoveObject {
                item_id: id,
                target_layer_index: 0,
                new_start: RationalTime::from_seconds(sec),
            },
            CommandSource::Gui,
            Some("drag_move"),
        )
        .unwrap();
    }
    assert_eq!(bus.undo_len(), 2); // AddObject + 統合されたMoveObject

    // GUIドラッグ中はMCP操作をブロック
    bus.set_gui_drag_locked(true);
    let err = bus
        .execute(
            &mut project,
            EditCommand::DeleteObject { item_id: id },
            CommandSource::Mcp,
            None,
        )
        .unwrap_err();
    assert!(matches!(err, CommandError::GuiDragLocked));
    bus.set_gui_drag_locked(false);

    // Undoで元の位置 (0.0s) に戻る
    assert!(bus.undo(&mut project));
    assert_eq!(
        project.active_scene().unwrap().layers[0].items[0].start,
        RationalTime::ZERO
    );
    assert!(bus.redo(&mut project));
    assert_eq!(
        project.active_scene().unwrap().layers[0].items[0].start,
        RationalTime::from_seconds(2.0)
    );
}

#[test]
fn split_and_crash_recovery_from_history_jsonl() {
    let tmp = tempdir().unwrap();
    let base_project = Project::new("RecoverTest", ProjectSettings::default());
    let folder = ProjectFolder::init(tmp.path().join("proj"), &base_project).unwrap();

    let mut live_project = base_project.clone();
    let mut bus = CommandBus::with_folder(folder.clone());
    let item = sample_item("SplitMe");
    let id = item.id;
    let right_id = Uuid::new_v4();

    bus.execute(
        &mut live_project,
        EditCommand::Batch {
            label: "Add and Split".into(),
            commands: vec![
                EditCommand::AddObject {
                    layer_index: 0,
                    item,
                },
                EditCommand::SplitObject {
                    item_id: id,
                    split_at: RationalTime::from_seconds(1.0),
                    new_right_id: right_id,
                },
            ],
        },
        CommandSource::Mcp,
        None,
    )
    .unwrap();
    assert_eq!(
        live_project.active_scene().unwrap().layers[0].items.len(),
        2
    );

    // クラッシュ想定: project.json は初期状態のまま、history.jsonl をリプレイして完全復旧
    let mut recovered = folder.load_project().unwrap();
    let history = folder.read_history().unwrap();
    let replayed = CommandBus::replay_history(&mut recovered, &history).unwrap();
    assert_eq!(replayed, 1);
    assert_eq!(recovered, live_project);
}

#[test]
fn smart_precompose_and_keyframe_are_undoable() {
    let mut project = Project::new("PrecompTest", ProjectSettings::default());
    let mut bus = CommandBus::new();

    let mut a = sample_item("Layer0Rect");
    a.start = RationalTime::from_seconds(1.0);
    let mut b = sample_item("Layer1Rect");
    b.start = RationalTime::from_seconds(1.5);
    let a_id = a.id;
    let b_id = b.id;

    bus.execute(
        &mut project,
        EditCommand::Batch {
            label: "Seed".into(),
            commands: vec![
                EditCommand::AddObject {
                    layer_index: 0,
                    item: a,
                },
                EditCommand::AddObject {
                    layer_index: 1,
                    item: b,
                },
                EditCommand::SetKeyframe {
                    item_id: a_id,
                    property: "x".into(),
                    keyframe: Keyframe {
                        time: RationalTime::ZERO,
                        value: 0.0,
                        interpolation: Interpolation::Bezier(CubicBezier::EASE_IN_OUT),
                    },
                },
                EditCommand::SetKeyframe {
                    item_id: a_id,
                    property: "x".into(),
                    keyframe: Keyframe {
                        time: RationalTime::from_seconds(1.0),
                        value: 100.0,
                        interpolation: Interpolation::Linear,
                    },
                },
            ],
        },
        CommandSource::Gui,
        None,
    )
    .unwrap();

    // ワンクリック結合（Smart Precompose）
    let new_scene_id = Uuid::new_v4();
    let precomp_item_id = Uuid::new_v4();
    bus.execute(
        &mut project,
        EditCommand::PrecomposeSelection {
            item_ids: vec![a_id, b_id],
            new_scene_id,
            precomp_name: "Motion Comp 1".into(),
            precomp_item_id,
            target_layer_index: 0,
        },
        CommandSource::Gui,
        None,
    )
    .unwrap();

    assert_eq!(project.scenes.len(), 2);
    let main_scene = project.active_scene().unwrap();
    assert_eq!(main_scene.layers[0].items.len(), 1);
    assert_eq!(main_scene.layers[1].items.len(), 0);
    assert_eq!(
        main_scene.layers[0].items[0].start,
        RationalTime::from_seconds(1.0)
    );

    let nested = project.scene(new_scene_id).unwrap();
    assert_eq!(nested.layers[0].items[0].start, RationalTime::ZERO);
    assert_eq!(
        nested.layers[1].items[0].start,
        RationalTime::from_seconds(0.5)
    );

    // Undo で元の2レイヤー構成へ完全復元
    assert!(bus.undo(&mut project));
    assert_eq!(project.scenes.len(), 1);
    assert_eq!(project.active_scene().unwrap().layers[0].items.len(), 1);
    assert_eq!(project.active_scene().unwrap().layers[1].items.len(), 1);
}

#[test]
fn undo_stack_respects_max_depth_and_increments_revision() {
    let mut project = Project::new("CapTest", ProjectSettings::default());
    let mut bus = CommandBus::new().with_max_undo_depth(3);
    assert_eq!(bus.revision(), 0);

    for i in 0..5 {
        bus.execute(
            &mut project,
            EditCommand::AddObject {
                layer_index: 0,
                item: sample_item(&format!("Item {i}")),
            },
            CommandSource::Gui,
            None,
        )
        .unwrap();
    }
    assert_eq!(bus.undo_len(), 3);
    assert_eq!(bus.revision(), 5);

    assert!(bus.undo(&mut project));
    assert_eq!(bus.revision(), 6);
    assert!(bus.redo(&mut project));
    assert_eq!(bus.revision(), 7);
}

#[test]
fn effect_chain_reorder_paste_and_audio_dsp_commands_are_undoable() {
    let mut project = Project::new("FxTest", ProjectSettings::default());
    let mut bus = CommandBus::new();
    let item = sample_item("Clip");
    let item_id = item.id;
    bus.execute(
        &mut project,
        EditCommand::AddObject {
            layer_index: 0,
            item,
        },
        CommandSource::Gui,
        None,
    )
    .unwrap();

    let eff_a = EffectNode {
        id: uuid::Uuid::new_v4(),
        effect_type: "blur".into(),
        enabled: true,
        params: BTreeMap::new(),
    };
    let eff_b = EffectNode {
        id: uuid::Uuid::new_v4(),
        effect_type: "mosaic".into(),
        enabled: true,
        params: BTreeMap::new(),
    };
    let eff_a_id = eff_a.id;
    bus.execute(
        &mut project,
        EditCommand::PasteEffects {
            item_id,
            effects: vec![eff_a, eff_b],
        },
        CommandSource::Gui,
        None,
    )
    .unwrap();

    bus.execute(
        &mut project,
        EditCommand::ReorderEffect {
            item_id,
            from_index: 0,
            to_index: 1,
        },
        CommandSource::Gui,
        None,
    )
    .unwrap();
    let chain = &project.active_scene().unwrap().layers[0].items[0].effect_chain;
    assert_eq!(chain[0].effect_type, "mosaic");
    assert_eq!(chain[1].effect_type, "blur");

    bus.execute(
        &mut project,
        EditCommand::ToggleEffect {
            item_id,
            effect_id: eff_a_id,
            enabled: false,
        },
        CommandSource::Gui,
        None,
    )
    .unwrap();
    assert!(!project.active_scene().unwrap().layers[0].items[0].effect_chain[1].enabled);

    bus.execute(
        &mut project,
        EditCommand::RemoveEffect {
            item_id,
            effect_id: eff_a_id,
        },
        CommandSource::Gui,
        None,
    )
    .unwrap();
    assert_eq!(
        project.active_scene().unwrap().layers[0].items[0]
            .effect_chain
            .len(),
        1
    );

    assert!(bus.undo(&mut project));
    assert_eq!(
        project.active_scene().unwrap().layers[0].items[0]
            .effect_chain
            .len(),
        2
    );
}

#[test]
fn set_scene_settings_rescales_items_and_is_undoable() {
    use crate::project::ScenePreset;
    use crate::time::FrameRate;

    let mut project = Project::new(
        "SceneSettings",
        ProjectSettings {
            width: 1280,
            height: 720,
            fps: FrameRate::FPS_60,
            ..Default::default()
        },
    );
    let mut bus = CommandBus::new();
    let mut item = sample_item("Box");
    item.transform.x = 640.0;
    item.transform.y = 360.0;
    bus.execute(
        &mut project,
        EditCommand::AddObject {
            layer_index: 0,
            item,
        },
        CommandSource::Gui,
        None,
    )
    .unwrap();

    let (sw, sh) = ScenePreset::Shorts9x16.dimensions();
    bus.execute(
        &mut project,
        EditCommand::SetSceneSettings {
            width: sw,
            height: sh,
            fps: FrameRate::FPS_30,
            rescale_items: true,
        },
        CommandSource::Gui,
        None,
    )
    .unwrap();

    let sc = project.active_scene().unwrap();
    assert_eq!((sc.width, sc.height), (1080, 1920));
    assert_eq!(sc.fps, FrameRate::FPS_30);
    // 元の中心 (640, 360) -> 新中心 (540, 960) へ正確にリフレームされる
    assert!((sc.layers[0].items[0].transform.x - 540.0).abs() < 0.5);
    assert!((sc.layers[0].items[0].transform.y - 960.0).abs() < 0.5);

    // Undo で 1280x720・60fps・元座標へ完全復元
    assert!(bus.undo(&mut project));
    let sc = project.active_scene().unwrap();
    assert_eq!((sc.width, sc.height), (1280, 720));
    assert_eq!(sc.fps, FrameRate::FPS_60);
    assert!((sc.layers[0].items[0].transform.x - 640.0).abs() < 0.5);
}

#[test]
fn filter_group_spectrum_and_marker_update_commands_are_undoable() {
    let mut project = Project::new("V071CmdTest", ProjectSettings::default());
    let mut bus = CommandBus::new();

    let filter_item = TimelineItem::new(
        "Filter",
        RationalTime::ZERO,
        RationalTime::from_seconds(2.0),
        ItemKind::FilterObject {
            effect_type: "brightness".into(),
            scope: FilterTargetScope::ImmediateLayer,
            params: BTreeMap::from([("gain".into(), 1.2)]),
        },
    );
    let group_item = TimelineItem::new(
        "Group",
        RationalTime::ZERO,
        RationalTime::from_seconds(2.0),
        ItemKind::GroupControl {
            target_layer_count: 1,
        },
    );
    let spec_item = TimelineItem::new(
        "Spectrum",
        RationalTime::ZERO,
        RationalTime::from_seconds(2.0),
        ItemKind::AudioSpectrum {
            linked_audio_layer: 3,
            width: 400.0,
            height: 120.0,
            bar_count: 16,
            color: RgbaColor::WHITE,
            peak_color: RgbaColor::BLACK,
            sensitivity: 1.0,
        },
    );
    let marker = TimelineMarker::new(RationalTime::from_seconds(5.0), "旧タイトル");
    let (fid, gid, sid, mid) = (filter_item.id, group_item.id, spec_item.id, marker.id);

    bus.execute(
        &mut project,
        EditCommand::Batch {
            label: "seed".into(),
            commands: vec![
                EditCommand::AddObject {
                    layer_index: 2,
                    item: filter_item,
                },
                EditCommand::AddObject {
                    layer_index: 2,
                    item: group_item,
                },
                EditCommand::AddObject {
                    layer_index: 1,
                    item: spec_item,
                },
                EditCommand::AddMarker { marker },
            ],
        },
        CommandSource::Gui,
        None,
    )
    .unwrap();

    bus.execute(
        &mut project,
        EditCommand::SetFilterObject {
            item_id: fid,
            effect_type: "blur".into(),
            scope: FilterTargetScope::LayerRange { count: 2 },
            params: BTreeMap::from([("radius".into(), 6.0)]),
        },
        CommandSource::Gui,
        None,
    )
    .unwrap();
    bus.execute(
        &mut project,
        EditCommand::SetGroupControl {
            item_id: gid,
            target_layer_count: 3,
        },
        CommandSource::Gui,
        None,
    )
    .unwrap();
    let cyan = RgbaColor {
        r: 0.1,
        g: 0.8,
        b: 1.0,
        a: 1.0,
    };
    bus.execute(
        &mut project,
        EditCommand::SetAudioSpectrum {
            item_id: sid,
            linked_audio_layer: 2,
            width: 640.0,
            height: 180.0,
            bar_count: 32,
            color: cyan,
            peak_color: RgbaColor::WHITE,
            sensitivity: 2.5,
        },
        CommandSource::Gui,
        None,
    )
    .unwrap();
    bus.execute(
        &mut project,
        EditCommand::UpdateMarker {
            marker_id: mid,
            label: "本編ハイライト".into(),
            color: cyan,
        },
        CommandSource::Gui,
        None,
    )
    .unwrap();

    let sc = project.active_scene().unwrap();
    assert_eq!(sc.markers[0].label, "本編ハイライト");
    assert_eq!(sc.markers[0].color, cyan);
    assert!(matches!(
        &sc.layers[2].items[0].kind,
        ItemKind::FilterObject {
            effect_type,
            scope: FilterTargetScope::LayerRange { count: 2 },
            ..
        } if effect_type == "blur"
    ));
    assert!(matches!(
        sc.layers[2].items[1].kind,
        ItemKind::GroupControl {
            target_layer_count: 3
        }
    ));
    assert!(matches!(
        sc.layers[1].items[0].kind,
        ItemKind::AudioSpectrum {
            linked_audio_layer: 2,
            bar_count: 32,
            ..
        }
    ));

    // Undo 4 回で元の状態へ戻る
    for _ in 0..4 {
        assert!(bus.undo(&mut project));
    }
    let sc = project.active_scene().unwrap();
    assert_eq!(sc.markers[0].label, "旧タイトル");
    assert!(matches!(
        sc.layers[2].items[1].kind,
        ItemKind::GroupControl {
            target_layer_count: 1
        }
    ));
}
