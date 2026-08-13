//! Text files broken into records with `----` separators.
//! Reads 3+ hyphens for backward compatibility, writes 4 hyphens (asciidoc standard).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const RECORD_SEPARATOR: &str = "----";

/// A separator is a line of 3+ hyphens, optionally followed by trailing whitespace.
fn is_separator_line(line: &str) -> bool {
    let trimmed = line.trim_end();
    trimmed.len() >= 3 && trimmed.bytes().all(|b| b == b'-')
}

/// Read a file and return its records (separated by 3+ hyphens on a line) as plain strings.
/// A missing or unreadable file yields an empty list.
pub fn read_records(file_path: &Path) -> Vec<String> {
    match fs::read_to_string(file_path) {
        Ok(content) => split_records(&content),
        Err(_) => Vec::new(),
    }
}

/// Split content on separator lines, trimming each record and dropping empty ones.
pub fn split_records(content: &str) -> Vec<String> {
    let mut records = Vec::new();
    let mut current = String::new();
    for line in content.lines() {
        if is_separator_line(line) {
            push_trimmed(&mut records, &current);
            current.clear();
        } else {
            current.push_str(line);
            current.push('\n');
        }
    }
    push_trimmed(&mut records, &current);
    records
}

fn push_trimmed(records: &mut Vec<String>, text: &str) {
    let trimmed = text.trim();
    if !trimmed.is_empty() {
        records.push(trimmed.to_string());
    }
}

/// Indent any line that would read back as a record separator, so a hyphen
/// rule inside a description can't split its record on reload. Write-side
/// only, applied when a description enters a store: the indented form is the
/// stored form — nothing is unescaped on read.
pub fn indent_separator_lines(text: &str) -> String {
    text.lines()
        .map(|line| {
            if is_separator_line(line) {
                format!("  {line}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Join already-formatted record texts with the canonical record separator.
pub fn join_records<I>(record_texts: I) -> String
where
    I: IntoIterator<Item = String>,
{
    record_texts
        .into_iter()
        .collect::<Vec<_>>()
        .join(&format!("\n{RECORD_SEPARATOR}\n"))
}

/// Atomically write content, archiving the previous file into bak/ if one exists.
pub fn write_atomic(file_path: &Path, content: &str) -> io::Result<()> {
    archive(file_path)?;
    atomic_replace(file_path, content)
}

/// Atomically write content, replacing the previous file without keeping a copy.
pub fn write_atomic_no_backup(file_path: &Path, content: &str) -> io::Result<()> {
    atomic_replace(file_path, content)
}

fn timestamp() -> String {
    chrono::Local::now().format("%Y%m%d%H%M%S").to_string()
}

/// `tasks.txt` -> `tasks.20260813185300.txt`
fn timestamped_name(file_path: &Path) -> String {
    let stem = file_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let suffix = file_path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    format!("{stem}.{}{suffix}", timestamp())
}

fn parent_dir(file_path: &Path) -> PathBuf {
    file_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Move an existing file into a sibling bak/ directory with a timestamped name.
fn archive(file_path: &Path) -> io::Result<()> {
    if !file_path.exists() {
        return Ok(());
    }
    let bak_dir = parent_dir(file_path).join("bak");
    fs::create_dir_all(&bak_dir)?;
    fs::rename(file_path, bak_dir.join(timestamped_name(file_path)))
}

/// Write content to a temp sibling and atomically swap it into place.
fn atomic_replace(file_path: &Path, content: &str) -> io::Result<()> {
    fs::create_dir_all(parent_dir(file_path))?;
    let tmp_path = file_path.with_file_name(timestamped_name(file_path));
    fs::write(&tmp_path, content)?;
    fs::rename(&tmp_path, file_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_three_or_more_hyphens() {
        let content = "a\n---\nb\n----\nc\n-------\nd";
        assert_eq!(split_records(content), vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn separator_allows_trailing_whitespace_only() {
        assert_eq!(split_records("a\n----   \nb"), vec!["a", "b"]);
        // Leading whitespace means the line is not a separator.
        assert_eq!(split_records("a\n  ----\nb"), vec!["a\n  ----\nb"]);
    }

    #[test]
    fn two_hyphens_is_not_a_separator() {
        assert_eq!(split_records("a\n--\nb"), vec!["a\n--\nb"]);
    }

    #[test]
    fn drops_empty_records() {
        assert_eq!(split_records("----\n\n----\nx\n----"), vec!["x"]);
    }

    #[test]
    fn indent_separator_lines_defuses_only_separator_shaped_lines() {
        assert_eq!(
            indent_separator_lines("intro\n---\n----   \noutro"),
            "intro\n  ---\n  ----   \noutro"
        );
        // Not separators: text after the hyphens, existing indent, too short.
        assert_eq!(
            indent_separator_lines("--- text\n  ---\n--"),
            "--- text\n  ---\n--"
        );
        // An indented line survives a second pass unchanged.
        assert_eq!(
            indent_separator_lines(&indent_separator_lines("---")),
            "  ---"
        );
    }

    #[test]
    fn join_uses_four_hyphens() {
        let joined = join_records(vec!["a".to_string(), "b".to_string()]);
        assert_eq!(joined, "a\n----\nb");
    }
}
