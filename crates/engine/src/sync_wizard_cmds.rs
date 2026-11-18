//! Commands for the two sync workflows: the guided wizard and the advanced window.

use serde_json::Value;

use crate::file_cmds::object;
use crate::registry::CommandSpec;
use crate::sync_cmds::{bad, str_param, target};
use crate::sync_hub::{Arming, RowFilter, View, WIZARD_REVIEW};

const MENU: &[&str] = &["File", "Synchronize Folders"];

fn view() -> CommandSpec {
    CommandSpec {
        id: "sync.view",
        label: "Switch Sync Workflow",
        menu: MENU,
        shortcut: None,
        params: r#"{ id?: string, mode: "wizard"|"advanced" } — guided questions or every option at once"#,
        enabled: |s| s.sync.current_state().is_some_and(|st| !st.busy()),
        run: |s, params| {
            let id = target(s, params)?;
            let mode = match str_param(params, "mode")? {
                Some("wizard") => View::Wizard,
                Some("advanced") => View::Advanced,
                _ => return Err(bad("mode must be wizard or advanced")),
            };
            let st = s.sync.states.get_mut(&id).ok_or_else(|| bad("no sync"))?;
            if st.busy() {
                return Err(bad("wait for the running sync to finish"));
            }
            if st.arming != Arming::Off && !st.watching() {
                st.arming = Arming::Off;
            }
            st.view = mode;
            Ok(())
        },
    }
}

fn go() -> CommandSpec {
    CommandSpec {
        id: "sync.wizard.go",
        label: "Go to Sync Wizard Step",
        menu: MENU,
        shortcut: None,
        params: r#"{ id?: string, step: 0..=6 } — 0 folders, 1 goal, 2 deleting, 3 compare by, 4 filters, 5 keep in sync, 6 review (compares, changes nothing)"#,
        enabled: |s| s.sync.current_state().is_some_and(|st| !st.busy()),
        run: |s, params| {
            let id = target(s, params)?;
            let step = object(params)?
                .and_then(|m| m.get("step"))
                .and_then(Value::as_u64)
                .filter(|n| *n <= u64::from(WIZARD_REVIEW))
                .ok_or_else(|| bad("step must be a whole number from 0 to 6"))?;
            let step = step as u8;
            let st = s.sync.states.get_mut(&id).ok_or_else(|| bad("no sync"))?;
            if st.busy() {
                return Err(bad("wait for the running sync to finish"));
            }
            if step > st.step {
                st.profile.validate()?;
            }
            st.step = step;
            st.view = View::Wizard;
            if step < WIZARD_REVIEW {
                if !st.watching() {
                    st.arming = Arming::Off;
                }
                return Ok(());
            }
            st.filter = RowFilter::Changes;
            // The review compares; keeping in sync then waits for the user's go-ahead.
            let keep = st.profile.watch.enabled && !st.watching();
            if keep {
                st.arming = Arming::Comparing;
            }
            let started = s.sync_compare(&id, false);
            if started.is_err()
                && let Some(st) = s.sync.states.get_mut(&id)
            {
                st.arming = Arming::Off;
            }
            started
        },
    }
}

fn swap() -> CommandSpec {
    CommandSpec {
        id: "sync.swap",
        label: "Swap Sync Folders",
        menu: MENU,
        shortcut: None,
        params: r#"{ id?: string } — exchanges left and right (reverses a one-way sync)"#,
        enabled: |s| s.sync.current_state().is_some_and(|st| !st.busy()),
        run: |s, params| {
            let id = target(s, params)?;
            let st = s.sync.states.get_mut(&id).ok_or_else(|| bad("no sync"))?;
            if st.busy() || st.watching() {
                return Err(bad("stop the running sync before swapping its folders"));
            }
            let p = &mut st.profile;
            std::mem::swap(&mut p.left, &mut p.right);
            st.saved = false;
            st.arming = Arming::Off;
            st.plan = None;
            st.rows.clear();
            st.phase = crate::sync_hub::Phase::Idle;
            Ok(())
        },
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![view(), go(), swap()]
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    use crate::session::Session;
    use crate::sync_hub::{Arming, View};
    use serde_json::json;

    #[test]
    fn a_new_sync_opens_in_the_wizard_and_the_review_compares() {
        let a = tempfile::tempdir().expect("tmp");
        let b = tempfile::tempdir().expect("tmp");
        std::fs::write(a.path().join("x.txt"), "x").expect("write");
        let mut s = Session::new().expect("session");
        s.dispatch("sync.open", &json!({})).expect("open");
        assert_eq!(s.sync.current_state().expect("st").view, View::Wizard);
        s.dispatch(
            "sync.profile.set",
            &json!({"patch": {
                "left": a.path().to_string_lossy(),
                "right": b.path().to_string_lossy(),
            }}),
        )
        .expect("set");
        s.dispatch("sync.wizard.go", &json!({"step": 6}))
            .expect("review");
        for _ in 0..200 {
            s.poll();
            if s.sync.current_state().is_some_and(|st| st.plan.is_some()) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let st = s.sync.current_state().expect("st");
        assert_eq!(st.step, 6);
        assert!(st.plan.is_some());
        assert_eq!(st.arming, Arming::Off);

        s.dispatch("sync.view", &json!({"mode": "advanced"}))
            .expect("advanced");
        assert_eq!(s.sync.current_state().expect("st").view, View::Advanced);
    }

    #[test]
    fn the_review_waits_for_confirmation_when_keeping_in_sync() {
        let a = tempfile::tempdir().expect("tmp");
        let b = tempfile::tempdir().expect("tmp");
        let mut s = Session::new().expect("session");
        s.dispatch("sync.open", &json!({})).expect("open");
        s.dispatch(
            "sync.profile.set",
            &json!({"patch": {
                "left": a.path().to_string_lossy(),
                "right": b.path().to_string_lossy(),
                "watch": {"enabled": true, "poll_secs": 30},
            }}),
        )
        .expect("set");
        s.dispatch("sync.wizard.go", &json!({"step": 6}))
            .expect("review");
        for _ in 0..200 {
            s.poll();
            if s.sync
                .current_state()
                .is_some_and(|st| st.arming == Arming::Confirm)
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(s.sync.current_state().expect("st").arming, Arming::Confirm);
        // Going back abandons the pending start.
        s.dispatch("sync.wizard.go", &json!({"step": 5}))
            .expect("back");
        assert_eq!(s.sync.current_state().expect("st").arming, Arming::Off);
    }

    #[test]
    fn swapping_exchanges_the_folders_and_drops_the_comparison() {
        let a = tempfile::tempdir().expect("tmp");
        let b = tempfile::tempdir().expect("tmp");
        let mut s = Session::new().expect("session");
        s.dispatch("sync.open", &json!({})).expect("open");
        s.dispatch(
            "sync.profile.set",
            &json!({"patch": {
                "left": a.path().to_string_lossy(),
                "right": b.path().to_string_lossy(),
            }}),
        )
        .expect("set");
        let (l, r) = {
            let p = &s.sync.current_state().expect("st").profile;
            (p.left.clone(), p.right.clone())
        };
        s.dispatch("sync.swap", &json!({})).expect("swap");
        let st = s.sync.current_state().expect("st");
        assert_eq!((&st.profile.left, &st.profile.right), (&r, &l));
        assert!(st.plan.is_none());
    }

    #[test]
    fn bad_params_are_refused() {
        let mut s = Session::new().expect("session");
        s.dispatch("sync.open", &json!({})).expect("open");
        assert!(s.dispatch("sync.wizard.go", &json!({"step": 7})).is_err());
        assert!(s.dispatch("sync.wizard.go", &json!({"step": "x"})).is_err());
        assert!(s.dispatch("sync.wizard.go", &json!({})).is_err());
        assert!(s.dispatch("sync.view", &json!({"mode": "fancy"})).is_err());
        // Moving on needs two valid folders.
        assert!(s.dispatch("sync.wizard.go", &json!({"step": 1})).is_err());
    }
}
