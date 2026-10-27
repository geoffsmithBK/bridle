//! Path resolution for tool arguments.

use std::path::{Component, Path, PathBuf};

/// Resolve a user/model-supplied path against `cwd`, expanding a leading `~`,
/// and normalize `.` and `..` lexically (the file may not exist yet).
pub fn resolve(cwd: &Path, raw: &str) -> PathBuf {
    let expanded = if raw == "~" || raw.starts_with("~/") {
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(home).join(raw.trim_start_matches('~').trim_start_matches('/'))
    } else {
        PathBuf::from(raw)
    };
    let joined = if expanded.is_absolute() { expanded } else { cwd.join(expanded) };
    normalize(&joined)
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// True if `path` is not inside `root`.
pub fn is_outside(root: &Path, path: &Path) -> bool {
    !path.starts_with(root)
}
