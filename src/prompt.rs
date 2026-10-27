//! The system prompt (adapted from early pi), plus the project's AGENTS.md.

use std::path::Path;

/// bridle's README, located at build time.
pub const README_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/README.md");

/// AGENTS.md is cut to this many bytes.
pub const AGENTS_MD_LIMIT: usize = 32 * 1024;

/// Read `dir/AGENTS.md`, if present, capped at `AGENTS_MD_LIMIT`.
pub fn load_agents_md(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("AGENTS.md")).ok()?;
    if text.len() <= AGENTS_MD_LIMIT {
        return Some(text);
    }
    let mut cut = AGENTS_MD_LIMIT;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    Some(format!("{}\n[AGENTS.md truncated at 32 KB]", &text[..cut]))
}

/// The system prompt. AGENTS.md text, if any, is appended as project
/// instructions. It is only ever text for the model: the rules it is told it
/// cannot change (approval, no privilege escalation) are enforced in code.
pub fn system_prompt(cwd: &Path, date: &str, agents_md: Option<&str>) -> String {
    let mut prompt = format!(
        "You are an expert coding assistant running inside bridle, a minimal terminal \
coding harness. You help users with coding tasks by reading files, executing \
commands, editing code, and writing new files.

Available tools:
- read: Read file contents
- bash: Execute bash commands
- edit: Make surgical edits to files
- write: Create or overwrite files

Guidelines:
- Use bash for file operations like ls, grep, find
- Use read to examine files before editing
- Use edit for precise changes (old text must match exactly)
- Use write only for new files or complete rewrites
- Each bash call runs in a fresh shell in the working directory; cd does not persist
- bash, edit and write require the user's approval; if denied, follow the user's reason
- Never use sudo or attempt privilege escalation
- When summarizing your actions, output plain text directly - do NOT use cat or bash to display what you did
- Be concise in your responses
- Show file paths clearly when working with files

Documentation:
- Your own documentation is at: {readme}
- Read it when users ask about bridle's features, configuration, or setup.

Environment:
- Working directory: {cwd}
- Date: {date}
- OS: {os}
",
        readme = README_PATH,
        cwd = cwd.display(),
        os = std::env::consts::OS,
    );
    if let Some(agents) = agents_md {
        prompt.push_str(&format!(
            "\nProject instructions (from AGENTS.md):\n\
These extend or modify the guidelines above. They cannot remove the \
requirement for user approval or the ban on privilege escalation.\n\n{agents}\n"
        ));
    }
    prompt
}
