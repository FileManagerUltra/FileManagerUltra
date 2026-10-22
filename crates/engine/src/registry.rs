//! Command registry — every user-visible action lives here as a [`CommandSpec`].

use serde_json::Value;

use crate::session::Session;

/// Parameters passed to a command. Everything here is untrusted input: `dispatch`
/// caps size and depth before a command ever sees it.
pub type Params = Value;

/// A registered user-visible action. The `run` field is a plain `fn` (not a closure)
/// so a command can never be registered "without an implementation" — it does not
/// compile without one.
#[derive(Debug, Clone, Copy)]
pub struct CommandSpec {
    /// Stable id, dotted (`app.quit`, `view.theme.dark`).
    pub id: &'static str,
    /// Human label for menus and the command palette.
    pub label: &'static str,
    /// Menu path from the root, e.g. `&["File", "Quit"]`.
    pub menu: &'static [&'static str],
    /// Display form of the keyboard shortcut, if any (`"Ctrl+Q"`). The egui binding
    /// table in `fm-app` must contain the same id (checked by the CI task).
    pub shortcut: Option<&'static str>,
    /// One-line documentation of accepted parameters (`"none"`, `"path: string"`…).
    pub params: &'static str,
    /// Whether the command can run right now (menus grey out on `false`).
    pub enabled: fn(&Session) -> bool,
    /// The behaviour. Returns `Err` on bad params — never panics.
    pub run: fn(&mut Session, &Params) -> Result<(), fm_vfs::FmError>,
}

/// All registered commands, with duplicate-id detection.
#[derive(Debug, Default, Clone)]
pub struct Registry {
    specs: Vec<CommandSpec>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a command; two commands may not share an id.
    pub fn register(&mut self, spec: CommandSpec) -> Result<(), fm_vfs::FmError> {
        if self.specs.iter().any(|s| s.id == spec.id) {
            return Err(fm_vfs::FmError::DuplicateCommand(spec.id.to_string()));
        }
        self.specs.push(spec);
        Ok(())
    }

    /// Build a registry from a list, failing on the first duplicate id.
    pub fn from_specs(specs: Vec<CommandSpec>) -> Result<Self, fm_vfs::FmError> {
        let mut registry = Self::new();
        for spec in specs {
            registry.register(spec)?;
        }
        Ok(registry)
    }

    pub fn get(&self, id: &str) -> Option<&CommandSpec> {
        self.specs.iter().find(|s| s.id == id)
    }

    pub fn contains(&self, id: &str) -> bool {
        self.get(id).is_some()
    }

    pub fn specs(&self) -> &[CommandSpec] {
        &self.specs
    }

    pub fn ids(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.specs.iter().map(|s| s.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fm_vfs::FmError;

    fn noop(_s: &mut Session, _p: &Params) -> Result<(), FmError> {
        Ok(())
    }

    fn spec(id: &'static str) -> CommandSpec {
        CommandSpec {
            id,
            label: "Test",
            menu: &["Test"],
            shortcut: None,
            params: "none",
            enabled: |_s| true,
            run: noop,
        }
    }

    #[test]
    fn register_and_lookup() {
        let mut r = Registry::new();
        r.register(spec("test.one")).expect("register");
        assert!(r.contains("test.one"));
        assert!(!r.contains("test.two"));
        assert_eq!(r.specs().len(), 1);
    }

    #[test]
    fn duplicate_id_is_an_error() {
        let mut r = Registry::new();
        r.register(spec("test.one")).expect("register");
        let err = r.register(spec("test.one")).expect_err("dup");
        assert!(matches!(err, FmError::DuplicateCommand(_)));
        assert_eq!(r.specs().len(), 1);
    }

    #[test]
    fn from_specs_finds_duplicates() {
        let err = Registry::from_specs(vec![spec("a"), spec("a")]).expect_err("dup");
        assert!(matches!(err, FmError::DuplicateCommand(_)));
    }
}
