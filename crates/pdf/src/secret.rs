//! Passwords as secret values: cleared on drop, never printed.

use zeroize::Zeroizing;

/// A password held only in memory. `Debug` prints `[redacted]`; there is no `Display`,
/// so it cannot end up in an error text or a log line by accident.
#[derive(Clone, Default)]
pub struct Secret(Zeroizing<String>);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(Zeroizing::new(value.into()))
    }

    /// The secret text; use it right away, never store or format it.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[redacted]")
    }
}

impl PartialEq for Secret {
    fn eq(&self, other: &Self) -> bool {
        // Not constant-time: only used to tell whether a cached document matches.
        *self.0 == *other.0
    }
}

impl Eq for Secret {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_is_redacted() {
        let s = Secret::new("hunter2");
        assert_eq!(format!("{s:?}"), "[redacted]");
        assert_eq!(s.expose(), "hunter2");
    }
}
