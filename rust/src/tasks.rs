//! Task records and the store backing pjpd/tasks.txt (ToDo) and
//! pjpd/tasks_done.txt (Done archive, written without bak/ backups).

use std::cmp::Reverse;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::record::{parse_record, resolve_id, stamp, tag_from_id};
use crate::{ids, textrec};

pub const STATUS_TODO: &str = "ToDo";
pub const STATUS_DONE: &str = "Done";

#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    pub id: String,
    pub tag: String,
    pub priority: i64,
    pub status: String,
    pub description: String,
    pub created: Option<String>,
    pub updated: Option<String>,
}

impl Task {
    /// Parse a task from record text: property lines (`Priority:`, `Status:`,
    /// `ID:`, `Tag:`, `Created:`, `Updated:`) in any order, all remaining lines
    /// forming the description. Returns None only when a missing ID cannot be
    /// regenerated from an invalid tag.
    pub fn from_text(text: &str) -> Option<Task> {
        let parsed = parse_record(
            text,
            &["status", "id", "tag", "created", "updated"],
            &["priority"],
        );
        let id = resolve_id(parsed.property("id"), parsed.property("tag"), "task")?;
        let priority = parsed.numeric_property("priority").unwrap_or(1);
        let status = parsed
            .owned_property("status")
            .unwrap_or_else(|| STATUS_TODO.to_string());
        let created = parsed.owned_property("created");
        let updated = parsed.owned_property("updated");

        Some(Task {
            tag: tag_from_id(&id),
            id,
            priority,
            status,
            description: parsed.description,
            created,
            updated,
        })
    }

    pub fn is_done(&self) -> bool {
        self.status == STATUS_DONE
    }

    /// Set created timestamp if not already set, and refresh updated.
    pub fn stamp(&mut self) {
        stamp(&mut self.created, &mut self.updated);
    }

    /// Render back to on-disk record form (each property on its own line).
    pub fn to_text(&self) -> String {
        let mut lines = vec![
            format!("Priority: {:4}", self.priority),
            format!("Status: {}", self.status),
            format!("ID: {}", self.id),
        ];
        if let Some(created) = &self.created {
            lines.push(format!("Created: {created}"));
        }
        if let Some(updated) = &self.updated {
            lines.push(format!("Updated: {updated}"));
        }
        lines.push(self.description.trim().to_string());
        lines.join("\n")
    }

    /// Convert to the dictionary shape used in API responses.
    pub fn to_json(&self) -> Value {
        let mut map = Map::new();
        map.insert("id".to_string(), json!(self.id));
        map.insert("priority".to_string(), json!(self.priority));
        map.insert("status".to_string(), json!(self.status));
        map.insert("description".to_string(), json!(self.description));
        if let Some(created) = &self.created {
            map.insert("created".to_string(), json!(created));
        }
        if let Some(updated) = &self.updated {
            map.insert("updated".to_string(), json!(updated));
        }
        Value::Object(map)
    }
}

/// Partial update applied to an existing task; None fields are left unchanged.
#[derive(Debug, Clone)]
pub struct TaskPatch {
    pub description: Option<String>,
    pub priority: Option<i64>,
    pub status: Option<String>,
}

impl TaskPatch {
    fn apply_to(&self, task: &mut Task) {
        if let Some(description) = &self.description {
            task.description = description.clone();
        }
        if let Some(priority) = self.priority {
            task.priority = priority;
        }
        if let Some(status) = &self.status {
            task.status = status.clone();
        }
    }
}

/// Store for the single project's tasks, reloaded from disk on every operation.
pub struct TaskStore {
    pub project_dir: PathBuf,
    pub tasks_file: PathBuf,
    pub done_tasks_file: PathBuf,
}

impl TaskStore {
    pub fn new(project_dir: &Path) -> TaskStore {
        let pjpd_dir = project_dir.join("pjpd");
        TaskStore {
            project_dir: project_dir.to_path_buf(),
            tasks_file: pjpd_dir.join("tasks.txt"),
            done_tasks_file: pjpd_dir.join("tasks_done.txt"),
        }
    }

    /// Warning when a `pjpd/<dir_name>.txt` file from the old multi-project
    /// layout still exists.
    pub fn legacy_warning(&self) -> Option<String> {
        let dir_name = self.project_dir.file_name()?.to_string_lossy().into_owned();
        let legacy_file = self
            .project_dir
            .join("pjpd")
            .join(format!("{dir_name}.txt"));
        if !legacy_file.exists() || legacy_file == self.tasks_file {
            return None;
        }
        Some(format!(
            "Legacy project file '{dir_name}.txt' found in pjpd/. \
             This is from an older multi-project layout. Tasks are now stored in pjpd/tasks.txt. \
             Migrate any tasks from '{dir_name}.txt' into tasks.txt, then delete the legacy file."
        ))
    }

    /// Load all tasks from both the todo and done files.
    pub fn load(&self) -> Vec<Task> {
        [&self.tasks_file, &self.done_tasks_file]
            .iter()
            .flat_map(|path| textrec::read_records(path))
            .filter_map(|record| Task::from_text(&record))
            .collect()
    }

    /// Save todo tasks to tasks.txt (with backup) and done tasks to
    /// tasks_done.txt (no backup), each sorted by priority descending.
    fn save(&self, tasks: &[Task]) -> io::Result<()> {
        let sorted_by_priority = |keep_done: bool| {
            let mut group: Vec<&Task> = tasks.iter().filter(|t| t.is_done() == keep_done).collect();
            group.sort_by_key(|t| Reverse(t.priority));
            group
        };

        let todo = sorted_by_priority(false);
        let done = sorted_by_priority(true);

        let todo_content = textrec::join_records(todo.iter().map(|t| t.to_text()));
        textrec::write_atomic(&self.tasks_file, &todo_content)?;

        if !done.is_empty() || self.done_tasks_file.exists() {
            let done_content = textrec::join_records(done.iter().map(|t| t.to_text()));
            textrec::write_atomic_no_backup(&self.done_tasks_file, &done_content)?;
        }
        Ok(())
    }

    /// Add a new task, creating the pjpd directory and tasks file if needed.
    pub fn add(&self, description: &str, priority: i64, tag: &str) -> Result<Task, String> {
        self.ensure_files().map_err(|e| e.to_string())?;
        let mut task = Task {
            id: ids::generate_with_tag(tag)?,
            tag: tag.to_string(),
            priority,
            status: STATUS_TODO.to_string(),
            description: description.to_string(),
            created: None,
            updated: None,
        };
        task.stamp();

        let mut tasks = self.load();
        tasks.push(task.clone());
        self.save(&tasks).map_err(|e| e.to_string())?;
        Ok(task)
    }

    pub fn get(&self, task_id: &str) -> Option<Task> {
        self.load().into_iter().find(|t| t.id == task_id)
    }

    /// Apply a patch to an existing task. Returns Ok(None) when the ID is unknown.
    pub fn update(&self, task_id: &str, patch: &TaskPatch) -> Result<Option<Task>, String> {
        let mut tasks = self.load();
        let Some(task) = tasks.iter_mut().find(|t| t.id == task_id) else {
            return Ok(None);
        };

        patch.apply_to(task);
        task.stamp();

        let updated = task.clone();
        self.save(&tasks).map_err(|e| e.to_string())?;
        Ok(Some(updated))
    }

    fn ensure_files(&self) -> io::Result<()> {
        let pjpd_dir = self.project_dir.join("pjpd");
        fs::create_dir_all(&pjpd_dir)?;
        if !self.tasks_file.exists() {
            fs::write(&self.tasks_file, "")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_record() {
        let task = Task::from_text(
            "Priority:   7\nStatus: ToDo\nID: mytag-ab12\nCreated: 2026-01-01T00:00:00Z\nUpdated: 2026-01-02T00:00:00Z\nDo the thing\nacross two lines",
        )
        .unwrap();
        assert_eq!(task.id, "mytag-ab12");
        assert_eq!(task.tag, "mytag");
        assert_eq!(task.priority, 7);
        assert_eq!(task.status, "ToDo");
        assert_eq!(task.description, "Do the thing\nacross two lines");
        assert_eq!(task.created.as_deref(), Some("2026-01-01T00:00:00Z"));
        assert_eq!(task.updated.as_deref(), Some("2026-01-02T00:00:00Z"));
    }

    #[test]
    fn non_digit_priority_line_becomes_description() {
        let task = Task::from_text("Priority: high\nID: t-ab12\nreal text").unwrap();
        assert_eq!(task.priority, 1);
        assert_eq!(task.description, "Priority: high\nreal text");
    }

    #[test]
    fn missing_id_is_generated_from_tag() {
        let task = Task::from_text("Tag: fix\nsomething").unwrap();
        assert!(task.id.starts_with("fix-"));
        assert_eq!(task.tag, "fix");
    }

    #[test]
    fn missing_id_and_tag_falls_back_to_legacy_id() {
        let task = Task::from_text("just a description").unwrap();
        // Legacy IDs look like XX-XXXX-XX; the tag is then re-extracted from
        // the first segment, mirroring the Python parser.
        assert_eq!(task.id.len(), 10);
        assert_eq!(task.tag, task.id.split('-').next().unwrap());
    }

    #[test]
    fn to_text_pads_priority_to_width_four() {
        let mut task = Task::from_text("Priority: 5\nID: t-ab12\nx").unwrap();
        task.created = None;
        task.updated = None;
        assert_eq!(
            task.to_text(),
            "Priority:    5\nStatus: ToDo\nID: t-ab12\nx"
        );
    }

    #[test]
    fn roundtrips_through_text() {
        let original = Task::from_text(
            "Priority:  42\nStatus: Done\nID: tag-wxyz\nCreated: 2026-01-01T00:00:00Z\nUpdated: 2026-01-01T00:00:00Z\nline one\nline two",
        )
        .unwrap();
        let reparsed = Task::from_text(&original.to_text()).unwrap();
        assert_eq!(original, reparsed);
    }
}
