//! Preferred listing widths; zero restores content fitting without storing font metrics.

use fm_layout::{MAX_COL_WIDTH, MIN_COL_WIDTH};
use fm_vfs::FmError;

use crate::file_cmds::object;
use crate::registry::{CommandSpec, Params};

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "view.columns.resize",
        label: "Resize File Columns",
        menu: &["View", "Columns", "Resize"],
        shortcut: None,
        params: "{pane?: number, widths: [number, number, number]} (zero = fit content)",
        enabled: |s| s.focused_is_files(),
        run: |s, params| {
            let bad = || {
                FmError::InvalidParams("widths must contain three numbers: 0 or 40..=400".into())
            };
            let map = object(params)?.ok_or_else(bad)?;
            let values = map
                .get("widths")
                .and_then(Params::as_array)
                .ok_or_else(bad)?;
            if values.len() != 3 {
                return Err(bad());
            }
            let mut widths = [0.0; 3];
            for (dest, value) in widths.iter_mut().zip(values) {
                let value = value.as_f64().ok_or_else(bad)?;
                if !value.is_finite()
                    || (value != 0.0
                        && !(f64::from(MIN_COL_WIDTH)..=f64::from(MAX_COL_WIDTH)).contains(&value))
                {
                    return Err(bad());
                }
                *dest = value as f32;
            }
            let pane = match map.get("pane") {
                None => s.layout.focused(),
                Some(v) => v
                    .as_u64()
                    .ok_or_else(|| FmError::InvalidParams("pane must be a number".into()))?,
            };
            let state = s
                .layout
                .state_mut(pane)
                .ok_or_else(|| FmError::NotFound(format!("pane {pane}")))?;
            if !state.kind.is_files() {
                return Err(FmError::Unsupported("pane is not a file listing"));
            }
            state.col_widths = widths;
            Ok(())
        },
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Session;
    use serde_json::json;

    #[test]
    fn resize_and_fit_are_registered_and_invalid_params_leave_widths_unchanged() {
        let mut s = Session::new().expect("session");
        s.dispatch("view.columns.resize", &json!({"widths": [100, 180, 0]}))
            .expect("resize");
        let saved = s.layout.state(1).expect("pane").col_widths;
        assert_eq!(saved, [100.0, 180.0, 0.0]);
        for bad in [
            Params::Null,
            json!([]),
            json!({}),
            json!({"widths": [100, 180]}),
            json!({"widths": [100, 180, 0, 80]}),
            json!({"widths": [100, "180", 0]}),
            json!({"widths": [100, -1, 0]}),
            json!({"widths": [100, 39, 0]}),
            json!({"widths": [100, 401, 0]}),
            json!({"widths": [0, 0, 0], "pane": "1"}),
            json!({"widths": [0, 0, 0], "pane": 999}),
        ] {
            assert!(s.dispatch("view.columns.resize", &bad).is_err(), "{bad}");
            assert_eq!(s.layout.state(1).expect("pane").col_widths, saved);
        }
        s.dispatch("view.columns.resize", &json!({"widths": [0, 0, 0]}))
            .expect("fit");
        assert_eq!(s.layout.state(1).expect("pane").col_widths, [0.0; 3]);
        s.layout.state_mut(1).expect("pane").kind = fm_layout::PaneKind::Terminal;
        assert!(
            s.dispatch("view.columns.resize", &json!({"widths": [100, 180, 0]}))
                .is_err()
        );
    }
}
