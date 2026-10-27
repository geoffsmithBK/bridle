//! Diffs for approval previews, and applying a reviewed write to disk.

use similar::{ChangeTag, TextDiff};
use std::path::Path;

/// Unified diff of `old` → `new`, labeled with `path`.
pub fn unified_diff(path: &str, old: &str, new: &str) -> String {
    TextDiff::from_lines(old, new).unified_diff().context_radius(3).header(path, path).to_string()
}

/// (lines removed, lines added)
pub fn diff_stats(old: &str, new: &str) -> (usize, usize) {
    let diff = TextDiff::from_lines(old, new);
    let mut removed = 0;
    let mut added = 0;
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Delete => removed += 1,
            ChangeTag::Insert => added += 1,
            ChangeTag::Equal => {}
        }
    }
    (removed, added)
}

/// Write `new` to `path`, but only if the file still holds exactly
/// `expected_old` (or still doesn't exist, when `expected_old` is None).
/// This guarantees that what the user approved is what gets written.
pub fn apply_write(path: &Path, expected_old: Option<&str>, new: &str) -> Result<(), String> {
    let shown = path.display();
    let current = std::fs::read(path).ok();
    let unchanged = match (expected_old, &current) {
        (None, None) => true,
        (Some(old), Some(cur)) => old.as_bytes() == cur.as_slice(),
        _ => false,
    };
    if !unchanged {
        return Err(format!("{shown} changed since the change was proposed; read it again."));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    std::fs::write(path, new).map_err(|e| format!("cannot write {shown}: {e}"))
}
