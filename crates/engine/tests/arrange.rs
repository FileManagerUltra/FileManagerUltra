#![allow(clippy::expect_used, clippy::panic)]

use serde_json::json;

use fm_engine::Session;
use fm_layout::{Axis, PaneNode};

fn three() -> (Session, u64, u64, u64) {
    let mut s = Session::new().expect("session");
    s.dispatch("pane.split.horizontal", &json!(null))
        .expect("split");
    s.dispatch("pane.split.horizontal", &json!(null))
        .expect("split");
    let p = s.layout.panes();
    (s, p[0], p[1], p[2])
}

#[test]
fn move_dispatches_every_edge_and_centre() {
    for edge in ["left", "right", "top", "bottom", "center"] {
        let (mut s, a, _b, c) = three();
        s.dispatch("pane.move", &json!({"pane": a, "target": c, "edge": edge}))
            .expect(edge);
        let mut leaves = s.layout.panes();
        leaves.sort_unstable();
        assert_eq!(leaves.len(), 3, "{edge}");
        assert_eq!(s.layout.focused(), a, "{edge}");
        assert!(s.layout.state(a).is_some());
    }
}

#[test]
fn move_rejects_bad_params_and_changes_nothing() {
    let (mut s, a, _b, c) = three();
    let before = s.layout.root.clone();
    for p in [
        json!(null),
        json!({}),
        json!({"pane": a, "target": a, "edge": "left"}),
        json!({"pane": a, "target": 99, "edge": "left"}),
        json!({"pane": a, "target": c, "edge": "diagonal"}),
        json!({"pane": a, "target": c}),
        json!({"pane": "a", "target": c, "edge": "left"}),
        json!([1, 2]),
    ] {
        assert!(s.dispatch("pane.move", &p).is_err(), "{p}");
    }
    assert_eq!(s.layout.root, before);
}

#[test]
fn zoom_rotate_equalize_and_resize_commands() {
    let (mut s, a, _b, _c) = three();
    s.layout.focus_pane(a).expect("focus");
    s.dispatch("pane.resize.right", &json!({"amount": 0.1}))
        .expect("resize");
    let PaneNode::Split { ratio, .. } = &s.layout.root else {
        panic!("split");
    };
    assert!(
        (ratio - 0.6).abs() < 1e-5 || (ratio - 0.55).abs() < 1e-5,
        "{ratio}"
    );
    for bad in [
        json!({"amount": 0}),
        json!({"amount": 9}),
        json!({"amount": "x"}),
        json!(5),
    ] {
        assert!(s.dispatch("pane.resize.left", &bad).is_err(), "{bad}");
    }
    s.dispatch("pane.equalize", &json!(null)).expect("equalize");
    s.dispatch("pane.rotate", &json!(null)).expect("rotate");
    s.dispatch("pane.zoom", &json!(null)).expect("zoom");
    assert_eq!(s.layout.zoomed, Some(a));
    s.dispatch("pane.zoom", &json!(null)).expect("unzoom");
    assert_eq!(s.layout.zoomed, None);
    let _ = Axis::Horizontal;
}

#[test]
fn terminals_follow_a_moved_pane() {
    let mut s = Session::new().expect("session");
    s.dispatch("pane.split.horizontal", &json!(null))
        .expect("split");
    let panes = s.layout.panes();
    let (a, b) = (panes[0], panes[1]);
    s.layout.focus_pane(a).expect("focus");
    if s.dispatch("pane.kind.set", &json!({"kind": "terminal"}))
        .is_err()
    {
        return;
    }
    s.dispatch(
        "pane.move",
        &json!({"pane": a, "target": b, "edge": "bottom"}),
    )
    .expect("move");
    assert!(s.terminal(a).is_some(), "the shell moved with its pane");
    assert_eq!(s.pane_kind(a), Some(fm_layout::PaneKind::Terminal));
}
