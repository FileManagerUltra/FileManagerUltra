//! Turning the shown picture by quarter turns. Only the view turns: the file is never
//! changed, and the next file starts upright (after its own EXIF orientation).

use fm_preview::Preview;
use fm_vfs::FmError;

use crate::preview_cmds::{Body, PreviewSlot};
use crate::{CommandSpec, Params, Session};

fn bad(message: &str) -> FmError {
    FmError::InvalidParams(message.into())
}

fn shows_picture(slot: &PreviewSlot) -> bool {
    slot.subject.is_some()
        && matches!(&slot.body, Body::Static(p) if matches!(&**p, Preview::Image(_)))
}

fn any_picture(s: &Session) -> bool {
    s.previews.slots.values().any(shows_picture)
}

fn rotate(s: &mut Session, params: &Params) -> Result<(), FmError> {
    let map = params
        .as_object()
        .ok_or_else(|| bad("expected {direction: \"left\"|\"right\", pane?: number}"))?;
    if map.keys().any(|key| key != "direction" && key != "pane") {
        return Err(bad("unknown parameter"));
    }
    if map.get("pane").is_some_and(|p| p.as_u64().is_none()) {
        return Err(bad("pane must be a non-negative integer"));
    }
    let step = match map.get("direction").and_then(Params::as_str) {
        Some("right") => 1,
        Some("left") => 3,
        _ => return Err(bad("direction must be left or right")),
    };
    let pane = s.preview_pane(params)?;
    let slot = s
        .previews
        .slots
        .get_mut(&pane)
        .filter(|slot| shows_picture(slot))
        .ok_or_else(|| bad("the preview does not show a picture"))?;
    let uri = slot
        .subject
        .as_ref()
        .map(|subject| subject.uri.clone())
        .ok_or_else(|| bad("the preview does not show a picture"))?;
    let turns = (slot.image_turns() + step) % 4;
    slot.turn = Some((uri, turns));
    Ok(())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "preview.image.rotate",
        label: "Rotate Preview Picture",
        menu: &["View", "Preview", "Rotate Picture"],
        shortcut: None,
        params: "{direction: \"left\"|\"right\", pane?: number}: turn the shown picture a quarter turn; the file is not changed",
        enabled: any_picture,
        run: rotate,
    }]
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fm_layout::PaneKind;
    use fm_preview::ImageDoc;
    use fm_vfs::{Entry, EntryKind, VfsUri};
    use serde_json::json;

    use super::*;

    fn picture(s: &mut Session, pane: fm_layout::PaneId, path: &str) {
        let slot = s
            .previews
            .slots
            .entry(pane)
            .or_insert_with(PreviewSlot::new);
        slot.subject = Some(Entry::new(
            VfsUri::parse(path).expect("uri"),
            EntryKind::File,
        ));
        slot.body = Body::Static(Arc::new(Preview::Image(ImageDoc {
            width: 2,
            height: 1,
            frames: vec![(vec![0; 8], 0)],
            original: (2, 1),
            format: "JPEG",
        })));
    }

    fn session() -> (Session, fm_layout::PaneId) {
        let mut s = Session::new().expect("session");
        let pane = s.layout.focused();
        s.layout.state_mut(pane).expect("pane").kind = PaneKind::Preview;
        (s, pane)
    }

    fn turns(s: &Session, pane: fm_layout::PaneId) -> u8 {
        s.preview_slot(pane).map_or(0, PreviewSlot::image_turns)
    }

    #[test]
    fn turns_go_round_both_ways_and_reset_for_the_next_file() {
        let (mut s, pane) = session();
        picture(&mut s, pane, "/photos/a.jpg");
        assert!(s.command_enabled("preview.image.rotate"));
        let right = json!({"direction": "right"});
        let left = json!({"direction": "left"});
        s.dispatch("preview.image.rotate", &right).expect("right");
        assert_eq!(turns(&s, pane), 1);
        for _ in 0..3 {
            s.dispatch("preview.image.rotate", &right).expect("right");
        }
        assert_eq!(turns(&s, pane), 0, "four quarter turns are a full turn");
        s.dispatch("preview.image.rotate", &left).expect("left");
        assert_eq!(turns(&s, pane), 3);
        picture(&mut s, pane, "/photos/b.jpg");
        assert_eq!(turns(&s, pane), 0, "the next file starts upright");
        picture(&mut s, pane, "/photos/a.jpg");
        assert_eq!(turns(&s, pane), 3, "coming back keeps the turn");
    }

    #[test]
    fn bad_params_and_non_pictures_are_errors() {
        let (mut s, pane) = session();
        assert!(!s.command_enabled("preview.image.rotate"));
        assert!(
            s.dispatch("preview.image.rotate", &json!({"direction": "right"}))
                .is_err(),
            "nothing shown"
        );
        picture(&mut s, pane, "/photos/a.jpg");
        for bad in [
            json!(null),
            json!("right"),
            json!({}),
            json!({"direction": "up"}),
            json!({"direction": 1}),
            json!({"direction": "right", "pane": -1}),
            json!({"direction": "right", "pane": 999_999}),
            json!({"direction": "right", "angle": 90}),
        ] {
            assert!(s.dispatch("preview.image.rotate", &bad).is_err(), "{bad}");
        }
        assert_eq!(turns(&s, pane), 0);
    }
}
