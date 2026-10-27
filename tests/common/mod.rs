#![allow(dead_code)]

use bridle::tools::{ToolContext, guard};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A fresh, empty directory under the system temp dir.
pub fn temp_dir() -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "bridle-test-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // Canonical form, so paths compare equal (macOS /var -> /private/var).
    dir.canonicalize().unwrap()
}

/// A tool context rooted in a fresh temp dir.
pub fn context() -> ToolContext {
    let dir = temp_dir();
    ToolContext { cwd: dir.clone(), project_root: dir, shim_dir: guard::make_shim_dir().unwrap() }
}
