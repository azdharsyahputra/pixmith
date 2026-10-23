//! Frame, layer and tag edits through the safe `Doc` methods the timeline uses.

use pixmith_desktop::doc::Doc;
use pixmith_desktop::ui::timeline::TagInfo;

#[test]
fn add_and_remove_frames() {
    let mut doc = Doc::new(8, 8);
    assert_eq!(doc.frame_count(), 1);
    let f = doc.add_frame(0, false);
    assert_eq!((f, doc.frame_count()), (1, 2));
    let f = doc.add_frame(0, true);
    assert_eq!((f, doc.frame_count()), (1, 3));
    doc.set_frame_duration(1, 250);
    assert_eq!(doc.frame_duration(1), 250);
    assert!(doc.move_frame(1, 2));
    assert_eq!(doc.frame_duration(2), 250);
    assert!(doc.remove_frame(2));
    assert!(doc.remove_frame(0));
    assert_eq!(doc.frame_count(), 1);
    assert!(!doc.remove_frame(0), "the last frame stays");
}

#[test]
fn add_remove_and_group_layers() {
    let mut doc = Doc::new(8, 8);
    assert_eq!(doc.layer_count(), 1);
    let top = doc.add_layer(0);
    assert_eq!((top, doc.layer_count()), (1, 2));
    doc.rename_layer(top, "Top");
    assert_eq!(doc.layer_name(top), "Top");
    doc.set_layer_visible(top, false);
    assert!(!doc.layer_visible(top));
    let group = doc.wrap_in_group(top).expect("group");
    assert!(doc.layer_is_group(group));
    assert_eq!(doc.layer_level(group + 1), 1);
    assert_eq!(doc.cel_grid().len(), (doc.layer_count() * doc.frame_count()) as usize);
    assert!(doc.remove_layer(group), "removing a group removes its children");
    assert_eq!(doc.layer_count(), 1);
    assert!(!doc.remove_layer(0), "the last drawable layer stays");
    assert!(doc.is_dirty());
}

#[test]
fn tags_round_trip_and_undo() {
    let mut doc = Doc::new(8, 8);
    doc.add_frame(0, false);
    let tag = TagInfo { name: "walk".into(), from: 0, to: 1, direction: 2, repeat: 3, color: 0x22C55E };
    assert!(doc.add_tag(&tag));
    assert_eq!(doc.tag_count(), 1);
    assert_eq!(doc.tag(0), Some(tag.clone()));
    let edited = TagInfo { name: "run".into(), to: 0, ..tag };
    assert!(doc.update_tag(0, &edited));
    assert_eq!(doc.tag(0), Some(edited));
    assert!(doc.remove_tag(0));
    assert_eq!(doc.tag_count(), 0);
    assert!(doc.undo());
    assert_eq!(doc.tag_count(), 1);
}

#[test]
fn playback_follows_the_tag() {
    use pixmith_core::document::layers::PlayState;
    let mut doc = Doc::new(8, 8);
    for _ in 0..3 {
        doc.add_frame(0, false);
    }
    doc.add_tag(&TagInfo { name: "t".into(), from: 1, to: 2, direction: 0, repeat: 0, color: 0 });
    let mut cursor = PlayState::default();
    assert_eq!(doc.play_next(0, &mut cursor, true), 1);
    assert_eq!(doc.play_next(1, &mut cursor, true), 2);
    assert_eq!(doc.play_next(2, &mut cursor, true), 1, "loops inside the tag");
    assert_eq!(doc.play_next(2, &mut cursor, false), 3, "ignores tags when not looping");
}
