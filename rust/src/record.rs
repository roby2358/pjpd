//! The shared shape of task and idea records: a leading run of recognized
//! `Key: value` property lines (in any order), with everything from the
//! first non-property line onward forming the description.

use std::collections::BTreeMap;

use crate::ids;

/// Timestamp format used in Created/Updated fields (ISO-8601 UTC).
const ISO_FMT: &str = "%Y-%m-%dT%H:%M:%SZ";

pub fn now_utc() -> String {
    chrono::Utc::now().format(ISO_FMT).to_string()
}

/// Set created if missing and refresh updated, both to now.
pub fn stamp(created: &mut Option<String>, updated: &mut Option<String>) {
    let now = now_utc();
    if created.as_deref().unwrap_or("").is_empty() {
        *created = Some(now.clone());
    }
    *updated = Some(now);
}

/// Recognized properties plus the description assembled from all other lines.
pub struct ParsedRecord {
    properties: BTreeMap<String, String>,
    pub description: String,
}

impl ParsedRecord {
    pub fn property(&self, key: &str) -> Option<&str> {
        self.properties.get(key).map(String::as_str)
    }

    pub fn owned_property(&self, key: &str) -> Option<String> {
        self.properties.get(key).cloned()
    }

    pub fn numeric_property(&self, key: &str) -> Option<i64> {
        self.property(key).and_then(|v| v.parse().ok())
    }
}

/// Parse record text. Properties are consumed only from the leading run of
/// recognized property lines; the first non-property line ends the header and
/// everything from it onward is description, so property-shaped text inside a
/// description can never override the header. Keys are matched
/// case-insensitively; within the header a later duplicate overrides an
/// earlier one. `numeric_keys` are only recognized when their value is all
/// digits — a line like "Priority: high" ends the header.
pub fn parse_record(text: &str, string_keys: &[&str], numeric_keys: &[&str]) -> ParsedRecord {
    let lines: Vec<&str> = text.trim().split('\n').collect();
    let header = interpret_header(header_candidates(&lines), string_keys, numeric_keys);
    let description = lines[header.len()..].join("\n").trim().to_string();

    ParsedRecord {
        properties: header.into_iter().collect(),
        description,
    }
}

/// Syntax pass: the leading run of `Key: value`-shaped lines, split into
/// (lowercased key, trimmed value) pairs with no judgment about what the keys
/// mean.
fn header_candidates<'a>(lines: &[&'a str]) -> Vec<(String, &'a str)> {
    lines
        .iter()
        .map_while(|line| property_of(line.trim()))
        .collect()
}

/// Interpretation pass: consume candidates up to the first one that isn't a
/// recognized property. That line and everything after it belong to the
/// description, so the returned length is also the header's line count.
fn interpret_header(
    candidates: Vec<(String, &str)>,
    string_keys: &[&str],
    numeric_keys: &[&str],
) -> Vec<(String, String)> {
    candidates
        .into_iter()
        .take_while(|(key, value)| {
            string_keys.contains(&key.as_str())
                || (numeric_keys.contains(&key.as_str()) && is_digits(value))
        })
        .map(|(key, value)| (key, value.to_string()))
        .collect()
}

/// Append the description to rendered header lines, always separated by a
/// blank line: the blank line ends the header run on reload, so no
/// description line can ever be read back as a property line, and the parser
/// trims it back out of the description. Empty descriptions add nothing.
pub fn push_description(lines: &mut Vec<String>, description: &str) {
    let trimmed = description.trim();
    if !trimmed.is_empty() {
        lines.push(String::new());
        lines.push(trimmed.to_string());
    }
}

/// Resolve a record's ID: keep an explicit non-empty one, else generate from
/// the tag, else generate a legacy ID. None when the tag is present but invalid
/// (the whole record is then dropped, mirroring the Python parser).
pub fn resolve_id(explicit: Option<&str>, tag: Option<&str>, kind: &str) -> Option<String> {
    if let Some(id) = explicit.filter(|id| !id.is_empty()) {
        return Some(id.to_string());
    }
    let id = match tag.filter(|tag| !tag.is_empty()) {
        Some(tag) => ids::generate_with_tag(tag).ok()?,
        None => ids::generate_legacy(),
    };
    eprintln!("WARN: generated missing {kind} ID for malformed record: {id}");
    Some(id)
}

/// Split a stripped line into a lowercased property key and trimmed value.
/// Returns None when the line has no colon.
fn property_of(stripped: &str) -> Option<(String, &str)> {
    let (key, value) = stripped.split_once(':')?;
    Some((key.trim().to_lowercase(), value.trim()))
}

/// True when the value is a non-empty run of ASCII digits (mirrors Python str.isdigit).
fn is_digits(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit())
}

/// Extract the tag from an ID: the segment before the first hyphen when it is
/// at most 12 characters, otherwise "legacy".
pub fn tag_from_id(id: &str) -> String {
    if !id.contains('-') {
        return "legacy".to_string();
    }
    let first = id.split('-').next().unwrap_or("");
    if first.chars().count() <= 12 {
        first.to_string()
    } else {
        "legacy".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn property_lines_are_case_insensitive_and_trimmed() {
        let parsed = parse_record(
            "PRIORITY :  5 \nStatus: ToDo\nbody",
            &["status"],
            &["priority"],
        );
        assert_eq!(parsed.numeric_property("priority"), Some(5));
        assert_eq!(parsed.property("status"), Some("ToDo"));
        assert_eq!(parsed.description, "body");
    }

    #[test]
    fn non_digit_numeric_property_ends_header() {
        let parsed = parse_record("Priority: high\nbody", &[], &["priority"]);
        assert_eq!(parsed.numeric_property("priority"), None);
        assert_eq!(parsed.description, "Priority: high\nbody");
    }

    #[test]
    fn unrecognized_property_lines_stay_in_description() {
        let parsed = parse_record("note: remember\nbody", &["status"], &[]);
        assert_eq!(parsed.description, "note: remember\nbody");
    }

    #[test]
    fn later_duplicate_within_header_wins() {
        let parsed = parse_record("Status: ToDo\nStatus: Done", &["status"], &[]);
        assert_eq!(parsed.property("status"), Some("Done"));
    }

    #[test]
    fn property_shaped_line_after_description_start_is_not_consumed() {
        let parsed = parse_record(
            "Status: ToDo\nreproduce with input\nStatus: Done\nPriority: 99",
            &["status"],
            &["priority"],
        );
        assert_eq!(parsed.property("status"), Some("ToDo"));
        assert_eq!(parsed.numeric_property("priority"), None);
        assert_eq!(
            parsed.description,
            "reproduce with input\nStatus: Done\nPriority: 99"
        );
    }

    #[test]
    fn header_ends_at_first_non_property_line() {
        let parsed = parse_record(
            "Priority: high\nStatus: Done\nbody",
            &["status"],
            &["priority"],
        );
        // "Priority: high" is not a recognized property line, so the whole
        // record from there on is description — including the Status line.
        assert_eq!(parsed.property("status"), None);
        assert_eq!(parsed.description, "Priority: high\nStatus: Done\nbody");
    }

    #[test]
    fn resolve_id_keeps_explicit_id() {
        assert_eq!(
            resolve_id(Some("keep-ab29"), Some("other"), "task"),
            Some("keep-ab29".to_string())
        );
    }

    #[test]
    fn resolve_id_generates_from_tag_or_legacy() {
        assert!(
            resolve_id(None, Some("fix"), "task")
                .unwrap()
                .starts_with("fix-")
        );
        assert_eq!(resolve_id(Some(""), None, "task").unwrap().len(), 10);
        // Invalid tag: the record cannot be salvaged.
        assert_eq!(resolve_id(None, Some("far too long a tag"), "task"), None);
    }

    #[test]
    fn tag_comes_from_first_id_segment() {
        assert_eq!(tag_from_id("mytag-ab12"), "mytag");
        assert_eq!(tag_from_id("multi-part-ab12"), "multi");
        assert_eq!(tag_from_id("nodash"), "legacy");
        assert_eq!(tag_from_id("averyverylongfirstsegment-ab12"), "legacy");
    }
}
