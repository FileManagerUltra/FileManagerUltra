//! `Session` — the engine's application state and the single dispatch point.

use fm_layout::Layout;
use fm_vfs::{FmError, VfsUri};

use crate::commands;
use crate::jobs::JobQueue;
use crate::prefs::Prefs;
use crate::registry::{Params, Registry};

/// Cap: no command parameter payload may exceed this size (capability `security`).
pub const MAX_PARAMS_BYTES: usize = 1_048_576;
/// Cap: no parameter nesting may exceed this depth (capability `security`).
pub const MAX_PARAMS_DEPTH: u32 = 32;

/// Owns the command registry, pane layout, job queue, preferences and the flags the
/// UI observes. The UI dispatches into a `Session` and renders its state; it never
/// performs work itself.
#[derive(Debug)]
pub struct Session {
    pub registry: Registry,
    pub prefs: Prefs,
    pub jobs: JobQueue,
    pub layout: Layout,
    /// `app.quit` flips this; the eframe loop stops on it.
    pub running: bool,
    /// `app.about` raises this flag; the UI shows the dialog and clears it.
    pub about_open: bool,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            registry: Registry::new(),
            prefs: Prefs::default(),
            jobs: JobQueue::new(),
            layout: Layout::single(VfsUri::root_file()),
            running: true,
            about_open: false,
        }
    }
}

impl Session {
    /// A session with the full seed command catalogue registered.
    pub fn new() -> Result<Self, FmError> {
        Ok(Self {
            registry: Registry::from_specs(commands::specs())?,
            ..Default::default()
        })
    }

    /// The single entry point for all behaviour. Untrusted `params` are capped
    /// *before* any command sees them; an unknown id is an error, not a crash.
    pub fn dispatch(&mut self, id: &str, params: &Params) -> Result<(), FmError> {
        validate_params(params)?;
        let run = self
            .registry
            .get(id)
            .ok_or_else(|| FmError::UnknownCommand(id.to_string()))?
            .run;
        run(self, params)
    }

    /// `enabled` predicate for a command id (`false` when unknown).
    pub fn command_enabled(&self, id: &str) -> bool {
        self.registry
            .get(id)
            .is_some_and(|spec| (spec.enabled)(self))
    }
}

/// Validate an untrusted parameter payload against the caps. Total over any input:
/// an explicit stack, no recursion, early exit as soon as a cap is exceeded.
pub fn validate_params(params: &Params) -> Result<(), FmError> {
    // Depth 0 = the payload root; a value nested in N arrays has depth N.
    let mut stack: Vec<(&Params, u32)> = vec![(params, 0)];
    let mut size: usize = 0;
    while let Some((value, depth)) = stack.pop() {
        if depth > MAX_PARAMS_DEPTH {
            return Err(FmError::InvalidParams(
                "parameters nested too deeply".to_string(),
            ));
        }
        match value {
            Params::Null | Params::Bool(_) => size += 1,
            Params::Number(n) => size += n.to_string().len(),
            Params::String(s) => {
                size += s.len();
                if size > MAX_PARAMS_BYTES {
                    return Err(FmError::InvalidParams("parameters too large".to_string()));
                }
            }
            Params::Array(items) => {
                for item in items {
                    size += 2;
                    if size > MAX_PARAMS_BYTES {
                        return Err(FmError::InvalidParams("parameters too large".to_string()));
                    }
                    stack.push((item, depth + 1));
                }
            }
            Params::Object(map) => {
                for (key, item) in map {
                    size += key.len() + 4;
                    if size > MAX_PARAMS_BYTES {
                        return Err(FmError::InvalidParams("parameters too large".to_string()));
                    }
                    stack.push((item, depth + 1));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::CommandSpec;

    #[test]
    fn new_registers_the_seed_catalogue() {
        let session = Session::new().expect("session");
        for id in [
            "app.quit",
            "app.about",
            "view.theme.dark",
            "view.theme.light",
            "view.theme.system",
            "view.theme.toggle",
        ] {
            assert!(session.registry.contains(id), "missing command {id}");
        }
    }

    #[test]
    fn unknown_id_is_an_error_and_session_is_unchanged() {
        let mut session = Session::new().expect("session");
        let before = session.prefs.clone();
        let err = session
            .dispatch("does.not.exist", &Params::Null)
            .expect_err("unknown id");
        assert!(matches!(err, FmError::UnknownCommand(_)));
        assert!(err.to_string().contains("does.not.exist"));
        assert_eq!(session.prefs, before, "session must be unchanged");
        assert!(session.running);
    }

    #[test]
    fn empty_params_are_accepted_for_seed_commands() {
        let mut session = Session::new().expect("session");
        for id in session.registry.ids().collect::<Vec<_>>() {
            session
                .dispatch(id, &Params::Null)
                .unwrap_or_else(|e| panic!("dispatch({id}) failed: {e}"));
        }
    }

    #[test]
    fn params_over_one_mib_are_rejected_before_running() {
        let huge = Params::String("a".repeat(MAX_PARAMS_BYTES + 1));
        let err = validate_params(&huge).expect_err("too large");
        assert!(matches!(err, FmError::InvalidParams(_)));
    }

    #[test]
    fn nesting_over_32_is_rejected() {
        let mut deep = Params::Null;
        for _ in 0..(MAX_PARAMS_DEPTH + 1) {
            deep = Params::Array(vec![deep]);
        }
        let err = validate_params(&deep).expect_err("too deep");
        assert!(matches!(err, FmError::InvalidParams(_)));

        let mut ok = Params::Null;
        for _ in 0..MAX_PARAMS_DEPTH {
            ok = Params::Array(vec![ok]);
        }
        validate_params(&ok).expect("depth 32 is fine");
    }

    #[test]
    fn wide_but_small_arrays_are_fine() {
        let wide = Params::Array(vec![Params::Null; 1000]);
        validate_params(&wide).expect("small array");
    }

    #[test]
    fn command_enabled_is_total() {
        let session = Session::new().expect("session");
        assert!(session.command_enabled("app.quit"));
        assert!(!session.command_enabled("no.such.command"));
    }

    #[test]
    fn test_command_can_enqueue_a_job() {
        let mut session = Session::new().expect("session");
        session
            .registry
            .register(CommandSpec {
                id: "test.enqueue",
                label: "Test enqueue",
                menu: &["Test"],
                shortcut: None,
                params: "none",
                enabled: |_s| true,
                run: |s, _p| {
                    let id = s.jobs.submit(crate::jobs::JobLane::Local, |ctx| {
                        ctx.report(1, 1);
                        Ok(())
                    });
                    assert!(id > 0);
                    Ok(())
                },
            })
            .expect("register");
        session
            .dispatch("test.enqueue", &Params::Null)
            .expect("dispatch");
        // The job runs in the background; the dispatch call already returned.
        let terminal = session
            .jobs
            .wait_for_terminal(1, std::time::Duration::from_secs(5));
        assert!(matches!(terminal, Some(crate::jobs::JobEvent::Done(_))));
    }
}
