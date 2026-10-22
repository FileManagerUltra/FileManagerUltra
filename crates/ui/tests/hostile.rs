//! End-to-end hostile-fixture tests (capability `security`): a hostile tree
//! through the engine cache into the widgets must not panic, must cap rows,
//! and a capped read must surface as the pane error state.
#![allow(clippy::expect_used)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use fm_engine::Session;
use fm_testkit::MemFs;
use fm_ui::theme::Tokens;
use fm_ui::{listing, pane, tree};
use fm_vfs::VfsUri;

fn wait_ready(s: &mut Session, uri: &VfsUri) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        s.poll();
        if s.dirs.state(uri).is_ready()
            || matches!(s.dirs.state(uri), fm_engine::DirState::Failed { .. })
        {
            return;
        }
        assert!(Instant::now() < deadline, "read never settled");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn session_with(fs: MemFs, location: &str) -> (Session, VfsUri) {
    let mut s = Session::default();
    s.backends.register(Arc::new(fs));
    let uri = VfsUri::parse(location).expect("uri");
    s.layout.state_mut(1).expect("pane").location = uri.clone();
    s.submit_read(uri.clone()).expect("submit");
    wait_ready(&mut s, &uri);
    (s, uri)
}

#[allow(clippy::expect_used)]
fn render_frame(tokens: &Tokens, s: &mut Session) {
    let ctx = egui::Context::default();
    fm_ui::theme::apply_to(&ctx, tokens);
    let mut output = ctx.run_ui(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let _ = tree::show(ui, s, 1, tokens);
            let _ = listing::show(ui, s, 1, tokens, true);
            let _ = pane::show(ui, s, 1, tokens);
        });
    });
    output.textures_delta.clear();
}

#[test]
fn hostile_mems_tree_renders_without_panicking() {
    let fs = fm_testkit::hostile_tree("/hostile", 500);
    let (mut s, _uri) = session_with(fs, "/hostile");
    // Entries landed in the cache.
    let count = s
        .dirs
        .entries(&VfsUri::parse("/hostile").expect("uri"))
        .map(|e| e.len())
        .unwrap_or(0);
    assert!(count >= 500, "expected the hostile entries, got {count}");
    for tokens in [Tokens::dark(), Tokens::light()] {
        render_frame(&tokens, &mut s);
    }
}

#[test]
fn capped_local_read_surfaces_as_the_error_state() {
    let tmp = tempfile::tempdir().expect("tempdir");
    for i in 0..10 {
        std::fs::write(tmp.path().join(format!("f{i}.txt")), b"x").expect("write");
    }
    let mut s = Session::default();
    let capped = fm_fs_local::LocalFs::with_max_entries(3);
    s.backends.register(Arc::new(capped));
    let uri = VfsUri::parse(&format!("file://{}", tmp.path().display())).expect("uri");
    s.layout.state_mut(1).expect("pane").location = uri.clone();
    s.submit_read(uri.clone()).expect("submit");
    wait_ready(&mut s, &uri);
    assert!(
        matches!(s.dirs.state(&uri), fm_engine::DirState::Failed { .. }),
        "over-cap read must fail, not truncate"
    );
    for tokens in [Tokens::dark(), Tokens::light()] {
        render_frame(&tokens, &mut s);
    }
}
