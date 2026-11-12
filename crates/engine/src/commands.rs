//! Command catalogue — the one list the registry is built from.

use crate::app_cmds;
use crate::arrange_cmds;
use crate::clipboard_cmds;
use crate::dnd_cmds;
use crate::file_cmds;
use crate::goto_cmds;
use crate::job_cmds;
use crate::nav_cmds;
use crate::open_cmds;
use crate::pane_cmds;
use crate::pane_kind_cmds;
use crate::prefs_cmds;
use crate::preview_cmds;
use crate::registry::CommandSpec;
use crate::sort_cmds;
use crate::term_cmds;
use crate::theme_cmds;
use crate::thumb_cmds;
use crate::video_cmds;
use crate::view_cmds;
use crate::workspace_cmds;

/// Every seed command. New modules append here; the registry fails on duplicates.
pub fn specs() -> Vec<CommandSpec> {
    let mut specs = vec![
        app_cmds::quit(),
        app_cmds::about(),
        prefs_cmds::preferences(),
        prefs_cmds::retarget_links(),
        prefs_cmds::recursive_preview(),
        prefs_cmds::remote_images(),
        prefs_cmds::terminal_shell(),
        prefs_cmds::sftp_timing(),
        prefs_cmds::smb_encryption(),
        prefs_cmds::zoom_in(),
        prefs_cmds::zoom_out(),
        prefs_cmds::zoom_reset(),
        theme_cmds::dark(),
        theme_cmds::light(),
        theme_cmds::system(),
        theme_cmds::toggle(),
    ];
    specs.extend(crate::update_cmds::specs());
    specs.extend(crate::git_cmds::specs());
    specs.extend(crate::diff_cmds::specs());
    specs.extend(crate::diff_compare_cmds::specs());
    specs.extend(pane_cmds::specs());
    specs.extend(arrange_cmds::specs());
    specs.extend(pane_kind_cmds::specs());
    specs.extend(preview_cmds::specs());
    specs.extend(crate::preview_link_cmds::specs());
    specs.extend(crate::md_images::specs());
    specs.extend(crate::preview_text_cmds::specs());
    specs.extend(crate::preview_find_cmds::specs());
    specs.extend(crate::preview_code_cmds::specs());
    specs.extend(crate::preview_open_cmds::specs());
    specs.extend(crate::preview_scale_cmds::specs());
    specs.extend(crate::preview_filter_cmds::specs());
    specs.extend(crate::preview_order_cmds::specs());
    specs.extend(crate::preview_reload_cmds::specs());
    specs.extend(term_cmds::specs());
    specs.extend(crate::term_mouse_cmds::specs());
    specs.extend(crate::term_link_cmds::specs());
    specs.extend(nav_cmds::specs());
    specs.extend(open_cmds::specs());
    specs.extend(thumb_cmds::specs());
    specs.extend(video_cmds::specs());
    specs.extend(crate::tools_cmds::specs());
    specs.extend(crate::preview_audio_cmds::specs());
    specs.extend(crate::media_cmds::specs());
    specs.extend(crate::media_audio_cmds::specs());
    specs.extend(crate::media_entry_cmds::specs());
    specs.extend(crate::media_playlist_cmds::specs());
    specs.extend(crate::media_open_cmds::specs());
    specs.extend(crate::media_source_cmds::specs());
    specs.extend(crate::media_resume::specs());
    specs.extend(crate::media_group_cmds::specs());
    specs.extend(crate::media_book_cmds::specs());
    specs.extend(crate::media_video_cmds::specs());
    specs.extend(crate::media_track_cmds::specs());
    specs.extend(crate::media_subtitle_cmds::specs());
    specs.extend(crate::subtitle_style::specs());
    specs.extend(crate::media_fullscreen_cmds::specs());
    specs.extend(goto_cmds::specs());
    specs.extend(sort_cmds::specs());
    specs.extend(crate::column_cmds::specs());
    specs.extend(crate::log_prefs_cmds::specs());
    specs.extend(crate::display_prefs_cmds::specs());
    specs.extend(crate::preview_prefs_cmds::specs());
    specs.extend(view_cmds::specs());
    specs.extend(file_cmds::specs());
    specs.extend(clipboard_cmds::specs());
    specs.extend(job_cmds::specs());
    specs.extend(crate::messages::specs());
    specs.extend(dnd_cmds::specs());
    specs.extend(workspace_cmds::specs());
    specs.extend(crate::remote_cmds::specs());
    specs.extend(crate::network::specs());
    specs.extend(crate::trash_cmds::specs());
    specs.extend(crate::select_cmds::specs());
    specs.extend(crate::item_cmds::specs());
    specs.extend(crate::image_clip_cmds::specs());
    specs.extend(crate::places_cmds::specs());
    specs.extend(crate::devices_cmds::specs());
    specs.extend(crate::props_cmds::specs());
    specs.extend(crate::search_cmds::specs());
    specs.extend(crate::openwith_cmds::specs());
    specs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Params, Registry};
    use crate::session::Session;
    use fm_vfs::FmError;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    #[test]
    fn seed_catalogue_is_well_formed() {
        let specs = specs();
        assert!(specs.len() >= 6, "seed catalogue shrank");
        for spec in &specs {
            assert!(!spec.id.is_empty(), "empty id");
            assert!(!spec.label.is_empty(), "empty label for {}", spec.id);
            assert!(!spec.menu.is_empty(), "no menu path for {}", spec.id);
            assert!(!spec.params.is_empty(), "no params doc for {}", spec.id);
        }
        // Required by the command-registry spec.
        for required in [
            "app.quit",
            "app.about",
            "view.theme.dark",
            "view.theme.light",
            "view.theme.system",
            "view.theme.toggle",
        ] {
            assert!(
                specs.iter().any(|s| s.id == required),
                "missing seed command {required}"
            );
        }
    }

    #[test]
    fn ids_are_unique() {
        Registry::from_specs(specs()).expect("no duplicate ids");
    }

    #[test]
    fn every_command_survives_hostile_params() {
        // Payloads: empty, wrong-typed, oversized, absurdly deep.
        let huge = "a".repeat(crate::session::MAX_PARAMS_BYTES + 1);
        let mut deep = Params::Null;
        for _ in 0..40 {
            deep = Params::Array(vec![deep]);
        }
        let mut wrong_typed = serde_json::Map::new();
        wrong_typed.insert("theme".to_string(), Params::Bool(true));
        wrong_typed.insert("n".to_string(), Params::from(-1));
        wrong_typed.insert("s".to_string(), Params::String("x".repeat(100)));
        let payloads = [
            Params::Null,
            Params::Object(serde_json::Map::new()),
            Params::Array(vec![]),
            Params::String(huge),
            deep,
            Params::Object(wrong_typed),
        ];

        for spec in specs() {
            for (i, payload) in payloads.iter().enumerate() {
                let mut session = Session::new().expect("session");
                let result = catch_unwind(AssertUnwindSafe(|| session.dispatch(spec.id, payload)));
                let outcome = result.unwrap_or_else(|_| {
                    panic!("dispatch({}) panicked on payload #{i}", spec.id);
                });
                // Ok or Err are both fine; panics were caught above.
                match outcome {
                    Ok(()) | Err(_) => {}
                }
            }
        }
    }

    #[test]
    fn commands_never_panic_on_garbage_params_even_without_caps() {
        // Call run() directly (bypassing dispatch's caps) to prove each command
        // itself is defensive, as the spec requires.
        let garbage = [
            Params::Null,
            Params::Bool(true),
            Params::from(999),
            Params::String("password=hunter2".to_string()),
            Params::Array(vec![Params::Null, Params::Bool(false)]),
        ];
        for spec in specs() {
            for payload in &garbage {
                let mut session = Session::new().expect("session");
                let result = catch_unwind(AssertUnwindSafe(|| (spec.run)(&mut session, payload)));
                assert!(
                    result.is_ok(),
                    "run({}) panicked on garbage params",
                    spec.id
                );
            }
        }
    }

    #[test]
    fn run_errors_do_not_leak_credentials() {
        // Even a hypothetical error string built from params must be redacted on display.
        let params = Params::String("bob:s3cret@nas".to_string());
        let err = FmError::backend("smb", format!("bad input: {params}"));
        assert!(!err.to_string().contains("s3cret"), "{err}");
    }
}
