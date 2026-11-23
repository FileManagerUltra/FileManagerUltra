//! A sharper picture on demand: image previews are decoded with their longest side capped
//! (`Limits::image_edge`, 2048 px) because Fit rarely needs more. Shown at 100% or zoomed
//! in, that copy would be enlarged and blurry, so the Preview pane asks for the picture
//! again at the size it needs, up to what the GPU can hold.

use fm_vfs::FmError;

use crate::{CommandSpec, Params, Session, preview_cmds::has_preview};

/// Largest edge a sharper decode may ask for: 8192² RGBA is 256 MiB, enough for the
/// photos people take while keeping a hostile request bounded.
pub const MAX_EDGE: u32 = 8192;

fn bad_params() -> FmError {
    FmError::InvalidParams("expected {pane?: number, edge: number (pixels)}".into())
}

fn full(s: &mut Session, params: &Params) -> Result<(), FmError> {
    let map = params.as_object().ok_or_else(bad_params)?;
    if map.keys().any(|key| key != "pane" && key != "edge") {
        return Err(bad_params());
    }
    if map
        .get("pane")
        .is_some_and(|p| !p.is_null() && p.as_u64().is_none())
    {
        return Err(bad_params());
    }
    let edge = map
        .get("edge")
        .and_then(Params::as_u64)
        .and_then(|e| u32::try_from(e).ok())
        .filter(|e| *e > 0)
        .ok_or_else(bad_params)?
        .min(MAX_EDGE);
    let pane = s.preview_pane(params)?;
    s.preview_full_image(pane, edge).map(|_| ())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "preview.image.full",
        label: "Load Sharper Picture",
        menu: &["Preview", "Load Sharper Picture"],
        shortcut: None,
        params: "{pane?: number (default the focused Preview), edge: number (longest side in pixels, at most 8192)}",
        enabled: has_preview,
        run: full,
    }]
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn malformed_params_are_refused() {
        let mut s = Session::new().expect("session");
        for bad in [
            Params::Null,
            json!("x"),
            json!([]),
            json!({}),
            json!({"edge": 0}),
            json!({"edge": -5}),
            json!({"edge": "big"}),
            json!({"edge": u64::MAX}),
            json!({"pane": "a", "edge": 4096}),
            json!({"edge": 4096, "extra": 1}),
            json!({"pane": 999, "edge": 4096}),
        ] {
            assert!(full(&mut s, &bad).is_err(), "{bad} must be refused");
        }
    }
}
