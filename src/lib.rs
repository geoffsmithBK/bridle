//! bridle: a minimal coding harness for local models.
//!
//! The harness core (`agent`, `llm`, `tools`, `log`) never touches the
//! terminal. The UI (`ui`) never does HTTP or runs tools. They talk only
//! through the types in `events`.

pub mod config;
pub mod events;
pub mod llm;
pub mod log;
pub mod prompt;
pub mod time;
pub mod tools;
