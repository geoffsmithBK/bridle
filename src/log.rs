//! Session logs: a readable transcript (.md) and a wire log (.jsonl).

use serde_json::Value;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Writes logs under `<root>/.bridle/sessions/`. `SessionLog::disabled()`
/// writes nothing (used in tests).
pub struct SessionLog {
    md: Option<File>,
    jsonl: Option<File>,
    pub md_path: Option<PathBuf>,
}

impl SessionLog {
    pub fn disabled() -> Self {
        Self { md: None, jsonl: None, md_path: None }
    }

    /// Open new log files named `stamp.md` / `stamp.jsonl`.
    pub fn create(project_root: &Path, stamp: &str) -> std::io::Result<Self> {
        let base = project_root.join(".bridle");
        let dir = base.join("sessions");
        std::fs::create_dir_all(&dir)?;
        let ignore = base.join(".gitignore");
        if !ignore.exists() {
            std::fs::write(&ignore, "*\n")?;
        }
        let md_path = dir.join(format!("{stamp}.md"));
        Ok(Self {
            md: Some(File::create(&md_path)?),
            jsonl: Some(File::create(dir.join(format!("{stamp}.jsonl")))?),
            md_path: Some(md_path),
        })
    }

    /// Append text to the transcript. Logging failures are ignored: a full
    /// disk should not stop the session.
    pub fn md(&mut self, text: &str) {
        if let Some(f) = &mut self.md {
            let _ = writeln!(f, "{text}");
        }
    }

    /// Append one JSON record (one line) to the wire log.
    pub fn wire(&mut self, record: &Value) {
        if let Some(f) = &mut self.jsonl {
            let _ = writeln!(f, "{record}");
        }
    }
}
