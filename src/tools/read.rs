//! The `read` tool: numbered lines, paged.

use std::path::Path;

pub const MAX_LINES: usize = 2000;
pub const MAX_LINE_CHARS: usize = 2000;

/// Read `path` and format lines `offset..offset+limit` (1-based offset).
/// Errors are returned as the message the model should see.
pub fn read_file(path: &Path, offset: Option<usize>, limit: Option<usize>) -> Result<String, String> {
    let shown = path.display();
    if path.is_dir() {
        return Err(format!("{shown} is a directory. Use bash with ls to list it."));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {shown}: {e}"))?;
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return Err(format!("{shown} looks like a binary file."));
    }
    let text = String::from_utf8(bytes).map_err(|_| format!("{shown} is not valid UTF-8."))?;
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    if total == 0 {
        return Ok(format!("[{shown} is empty]"));
    }
    let start = offset.unwrap_or(1).max(1);
    if start > total {
        return Err(format!("offset {start} is past the end of {shown} ({total} lines)."));
    }
    let limit = limit.unwrap_or(MAX_LINES).clamp(1, MAX_LINES);
    let end = (start - 1 + limit).min(total);

    let mut out = String::new();
    for (i, line) in lines[start - 1..end].iter().enumerate() {
        let n = start + i;
        if line.chars().count() > MAX_LINE_CHARS {
            let cut: String = line.chars().take(MAX_LINE_CHARS).collect();
            out.push_str(&format!("{n:>6}│ {cut}… [line truncated]\n"));
        } else {
            out.push_str(&format!("{n:>6}│ {line}\n"));
        }
    }
    if end < total {
        out.push_str(&format!(
            "[lines {start}–{end} of {total} — call read again with offset={} to continue]\n",
            end + 1
        ));
    }
    Ok(out)
}
