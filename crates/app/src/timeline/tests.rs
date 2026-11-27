//! タイムライン操作単体テスト

use super::*;
use ve_core::{Project, ProjectSettings, ShapeKind};

#[test]
fn copy_paste_duplicate_and_select_all_preserve_offsets_and_undo() {
    let mut proj = Project::new("TlShortcutTest", ProjectSettings::default());
    let a = TimelineItem::new(
        "ClipA",
        RationalTime::from_seconds(1.0),
        RationalTime::from_seconds(2.0),
        ItemKind::Shape {
            shape: ShapeKind::Rectangle,
            width: 100.0,
            height: 60.0,
            color: RgbaColor::WHITE,
        },
    );
    let b = TimelineItem::new(
        "ClipB",
        RationalTime::from_seconds(2.5),
        RationalTime::from_seconds(1.0),
        ItemKind::Text {
            content: "字幕".into(),
            font_family: "sans-serif".into(),
            font_size: 40.0,
            color: RgbaColor::WHITE,
            animator: TextAnimator::default(),
        },
    );
    proj.active_scene_mut().unwrap().layers[0].items.push(a);
    proj.active_scene_mut().unwrap().layers[1].items.push(b);

    let mut sess = EditorSession::new(proj);
    let mut tl = TimelineState::default();

    tl.select_all_in_scene(&mut sess);
    assert_eq!(sess.selected_items.len(), 2);

    tl.copy_selected(&sess);
    assert_eq!(tl.clipboard_items.len(), 2);

    sess.playhead = RationalTime::from_seconds(10.0);
    tl.paste_clipboard_at_playhead(&mut sess);
    {
        let sc = sess.project.active_scene().unwrap();
        assert_eq!(sc.layers[0].items.len(), 2);
        assert_eq!(sc.layers[1].items.len(), 2);
        assert!((sc.layers[0].items[1].start.as_seconds() - 10.0).abs() < 1e-6);
        assert!((sc.layers[1].items[1].start.as_seconds() - 11.5).abs() < 1e-6);
        assert_ne!(sc.layers[0].items[0].id, sc.layers[0].items[1].id);
    }

    // Ctrl+D on newly pasted items duplicates them right after their combined end (12.5s)
    tl.duplicate_selected(&mut sess);
    {
        let sc = sess.project.active_scene().unwrap();
        assert_eq!(sc.layers[0].items.len(), 3);
        assert!((sc.layers[0].items[2].start.as_seconds() - 12.5).abs() < 1e-6);
    }

    assert!(sess.bus.undo(&mut sess.project));
    assert_eq!(
        sess.project.active_scene().unwrap().layers[0].items.len(),
        2
    );
}
