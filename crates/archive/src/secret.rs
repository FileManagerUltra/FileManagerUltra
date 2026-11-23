//! Archive passwords: zeroised on drop, never shown (security: "Archive passwords never
//! persist").

use std::fmt;

use zeroize::Zeroizing;

/// A password for an encrypted archive. `Debug` never shows it and it has no `Display`.
#[derive(Clone)]
pub struct Secret(Zeroizing<String>);

impl Secret {
    pub fn new(password: impl Into<String>) -> Self {
        Self(Zeroizing::new(password.into()))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<hidden>)")
    }
}

impl Eq for Secret {}

impl PartialEq for Secret {
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_shows_the_password() {
        let s = Secret::new("hunter2");
        assert!(!format!("{s:?}").contains("hunter2"));
        assert_eq!(s.expose(), "hunter2");
    }
}
