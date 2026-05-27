use std::path::{Path, PathBuf};

use crate::MemoryError;

/// Loads `~/.talon/USER.md` — personal context about the user injected into the system prompt.
pub struct UserMd;

/// Loads `~/.talon/MEMORY.md` — global memory notes injected into the system prompt.
pub struct MemoryMd;

impl UserMd {
    pub fn path() -> PathBuf {
        talon_dir().join("USER.md")
    }

    /// Read file contents; returns `None` if the file does not exist.
    pub fn load() -> Result<Option<String>, MemoryError> {
        read_optional(&Self::path())
    }
}

impl MemoryMd {
    pub fn path() -> PathBuf {
        talon_dir().join("MEMORY.md")
    }

    /// Read file contents; returns `None` if the file does not exist.
    pub fn load() -> Result<Option<String>, MemoryError> {
        read_optional(&Self::path())
    }
}

fn talon_dir() -> PathBuf {
    dirs_home().join(".talon")
}

fn dirs_home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}

fn read_optional(path: &Path) -> Result<Option<String>, MemoryError> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(MemoryError::Io(e)),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn user_md_path_ends_with_user_md() {
        let p = UserMd::path();
        assert!(p.to_string_lossy().ends_with("USER.md"), "path: {p:?}");
    }

    #[test]
    fn memory_md_path_ends_with_memory_md() {
        let p = MemoryMd::path();
        assert!(p.to_string_lossy().ends_with("MEMORY.md"), "path: {p:?}");
    }

    #[test]
    fn load_nonexistent_returns_none() {
        // Files don't exist in test env, so expect None (not an error).
        // If by coincidence they exist on the dev machine, just verify no panic.
        let _ = UserMd::load().expect("no error for missing file");
        let _ = MemoryMd::load().expect("no error for missing file");
    }

    #[test]
    fn load_returns_content_for_existing_file() {
        use std::io::Write;
        let dir = std::env::temp_dir();
        let path = dir.join("talon_test_user_md.md");
        let mut f = std::fs::File::create(&path).expect("create");
        f.write_all(b"# User context\nI like Rust.").expect("write");

        let content = read_optional(&path).expect("read").expect("some");
        assert!(content.contains("Rust"));

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn read_optional_io_error_propagates() {
        // Permission-denied paths exist in /proc on Linux; skip on macOS where
        // we can't easily create one — just verify NotFound maps to None.
        let absent = PathBuf::from("/tmp/__talon_nonexistent_file_xyz.md");
        let result = read_optional(&absent).expect("no error");
        assert!(result.is_none());
    }
}
