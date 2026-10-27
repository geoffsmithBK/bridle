//! Keeping bash away from privilege escalation. This is not a sandbox; the
//! user's approval of each command is the real protection. See spec §6.5.

use std::path::{Path, PathBuf};

const BLOCKED_WORDS: &[&str] = &["sudo", "su", "doas", "pkexec", "run0"];

/// Reject commands that mention privilege-escalation tools.
pub fn check_command(command: &str) -> Result<(), String> {
    let lower = command.to_lowercase();
    let mentions_word = lower
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        .any(|word| BLOCKED_WORDS.contains(&word));
    if mentions_word || lower.contains("with administrator privileges") {
        return Err("privilege escalation is not permitted in bridle.".into());
    }
    Ok(())
}

/// Create a directory holding a `sudo` that always fails. It is put first on
/// PATH for every command bridle runs.
pub fn make_shim_dir() -> std::io::Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("bridle-shim-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let shim = dir.join("sudo");
    std::fs::write(&shim, "#!/bin/sh\necho 'sudo is disabled inside bridle' >&2\nexit 1\n")?;
    std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755))?;
    Ok(dir)
}

/// PATH for child processes: the shim directory first.
pub fn child_path(shim_dir: &Path) -> String {
    let current = std::env::var("PATH").unwrap_or_default();
    format!("{}:{current}", shim_dir.display())
}
