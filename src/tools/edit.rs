//! The `edit` tool's matching logic: exact, unique replacement.

/// Return `content` with the single occurrence of `old` replaced by `new`.
/// Errors are the messages the model should see.
pub fn plan_edit(path: &str, content: &str, old: &str, new: &str) -> Result<String, String> {
    if old.is_empty() {
        return Err("old_text must not be empty. Use write to create a file.".into());
    }
    let starts: Vec<usize> = content.match_indices(old).map(|(i, _)| i).collect();
    match starts.len() {
        0 => Err(format!(
            "old_text not found in {path}. The file may have changed — read it again. \
             Check whitespace and indentation exactly."
        )),
        1 => Ok(content.replacen(old, new, 1)),
        n => {
            let lines: Vec<String> =
                starts.iter().map(|&i| (content[..i].matches('\n').count() + 1).to_string()).collect();
            Err(format!(
                "old_text matches {n} times (lines {}). Include more surrounding lines to make it unique.",
                lines.join(", ")
            ))
        }
    }
}
