//! Session files: a readable transcript (.md), a wire log (.jsonl), and the
//! saved history (.session.json) used to resume.

use crate::llm::types::Message;
use crate::session::{SavedSession, VERSION};
use serde_json::Value;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Writes a session's files under `<root>/.bridle/sessions/`: transcript,
/// wire log and saved history. `SessionLog::disabled()` writes nothing.
pub struct SessionLog {
    md: Option<File>,
    jsonl: Option<File>,
    pub md_path: Option<PathBuf>,
    /// The session's name; None when disabled.
    pub stamp: Option<String>,
    root: PathBuf,
}

impl SessionLog {
    pub fn disabled() -> Self {
        Self { md: None, jsonl: None, md_path: None, stamp: None, root: PathBuf::new() }
    }

    /// Start a new session's files named after `stamp` (with `-2`, `-3`, …
    /// added if that stamp is taken, e.g. by a `/new` in the same second).
    pub fn create(project_root: &Path, stamp: &str) -> std::io::Result<Self> {
        let base = project_root.join(".bridle");
        let dir = base.join("sessions");
        std::fs::create_dir_all(&dir)?;
        let ignore = base.join(".gitignore");
        if !ignore.exists() {
            std::fs::write(&ignore, "*\n")?;
        }
        let mut name = stamp.to_string();
        let mut n = 1;
        while dir.join(format!("{name}.md")).exists() {
            n += 1;
            name = format!("{stamp}-{n}");
        }
        Self::open(project_root, &name, false)
    }

    /// Reopen an existing session's files for appending.
    pub fn resume(project_root: &Path, stamp: &str) -> std::io::Result<Self> {
        Self::open(project_root, stamp, true)
    }

    fn open(project_root: &Path, stamp: &str, append: bool) -> std::io::Result<Self> {
        let dir = crate::session::sessions_dir(project_root);
        let open = |path: &Path| {
            std::fs::OpenOptions::new().create(true).append(append).write(true).truncate(!append).open(path)
        };
        let md_path = dir.join(format!("{stamp}.md"));
        Ok(Self {
            md: Some(open(&md_path)?),
            jsonl: Some(open(&dir.join(format!("{stamp}.jsonl")))?),
            md_path: Some(md_path),
            stamp: Some(stamp.to_string()),
            root: project_root.to_path_buf(),
        })
    }

    /// Save the history (without the system prompt) for `--continue`.
    /// Failures are ignored, like other logging.
    pub fn save_session(&mut self, cwd: &Path, messages: &[Message]) {
        if let Some(stamp) = &self.stamp {
            let saved =
                SavedSession { version: VERSION, cwd: cwd.to_path_buf(), messages: messages.to_vec() };
            let _ = crate::session::save(&crate::session::session_path(&self.root, stamp), &saved);
        }
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
