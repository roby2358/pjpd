//! Store-level tests: atomic writes, backups, done-file split, timestamps.

use pjpd::ideas::IdeaStore;
use pjpd::tasks::{TaskPatch, TaskStore};
use pjpd::textrec;
use tempfile::TempDir;

fn done_patch() -> TaskPatch {
    TaskPatch {
        description: None,
        priority: None,
        status: Some("Done".to_string()),
    }
}

#[test]
fn separator_lines_in_task_descriptions_are_indented_on_add() {
    let dir = TempDir::new().unwrap();
    let store = TaskStore::new(dir.path());

    let task = store.add("intro\n---\noutro", 5, "rule").unwrap();
    assert_eq!(task.description, "intro\n  ---\noutro");

    let loaded = store.load();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].description, "intro\n  ---\noutro");
}

#[test]
fn separator_lines_in_task_descriptions_are_indented_on_update() {
    let dir = TempDir::new().unwrap();
    let store = TaskStore::new(dir.path());

    let task = store.add("plain", 5, "rule").unwrap();
    let patch = TaskPatch {
        description: Some("before\n----\nafter".to_string()),
        priority: None,
        status: None,
    };
    let updated = store.update(&task.id, &patch).unwrap().unwrap();
    assert_eq!(updated.description, "before\n  ----\nafter");
    assert_eq!(store.load().len(), 1);
}

#[test]
fn separator_lines_in_idea_descriptions_are_indented() {
    let dir = TempDir::new().unwrap();
    let store = IdeaStore::new(dir.path());

    let idea = store.add("top\n---\nbottom", 5, "rule").unwrap();
    assert_eq!(idea.description, "top\n  ---\nbottom");

    let updated = store
        .update(&idea.id, "one\n-----\ntwo", 7)
        .unwrap()
        .unwrap();
    assert_eq!(updated.description, "one\n  -----\ntwo");
    assert_eq!(store.load().len(), 1);
}

#[test]
fn write_atomic_archives_previous_version() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("data.txt");

    textrec::write_atomic(&file, "v1").unwrap();
    textrec::write_atomic(&file, "v2").unwrap();

    assert_eq!(std::fs::read_to_string(&file).unwrap(), "v2");
    let bak_dir = dir.path().join("bak");
    let backups: Vec<String> = std::fs::read_dir(&bak_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(backups.len(), 1);
    assert!(backups[0].starts_with("data.") && backups[0].ends_with(".txt"));
    assert_eq!(
        std::fs::read_to_string(bak_dir.join(&backups[0])).unwrap(),
        "v1"
    );
}

#[test]
fn write_atomic_no_backup_keeps_no_copy() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("archive.txt");

    textrec::write_atomic_no_backup(&file, "v1").unwrap();
    textrec::write_atomic_no_backup(&file, "v2").unwrap();

    assert_eq!(std::fs::read_to_string(&file).unwrap(), "v2");
    assert!(!dir.path().join("bak").exists());
}

#[test]
fn done_tasks_split_into_done_file_without_backup() {
    let dir = TempDir::new().unwrap();
    let store = TaskStore::new(dir.path());

    let task = store.add("do it", 5, "split").unwrap();
    store.update(&task.id, &done_patch()).unwrap();

    let todo = std::fs::read_to_string(&store.tasks_file).unwrap();
    let done = std::fs::read_to_string(&store.done_tasks_file).unwrap();
    assert!(!todo.contains(&task.id));
    assert!(done.contains(&task.id));
    assert!(done.contains("Status: Done"));

    // The done file is an archive: rewriting it must not create backups of it.
    let bak_names: Vec<String> = std::fs::read_dir(dir.path().join("pjpd/bak"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(bak_names.iter().all(|n| !n.starts_with("tasks_done.")));
}

#[test]
fn update_preserves_created_and_refreshes_updated() {
    let dir = TempDir::new().unwrap();
    let store = TaskStore::new(dir.path());

    let task = store.add("stamped", 5, "time").unwrap();
    let created = task.created.clone().unwrap();
    assert_eq!(task.updated.as_deref(), Some(created.as_str()));

    let describe = TaskPatch {
        description: Some("new text".to_string()),
        priority: None,
        status: None,
    };
    let updated = store.update(&task.id, &describe).unwrap().unwrap();
    assert_eq!(updated.created.as_deref(), Some(created.as_str()));
    assert!(updated.updated.is_some());
}

#[test]
fn tasks_persist_sorted_by_priority_descending() {
    let dir = TempDir::new().unwrap();
    let store = TaskStore::new(dir.path());

    store.add("low", 1, "low").unwrap();
    store.add("high", 99, "high").unwrap();
    store.add("mid", 50, "mid").unwrap();

    let content = std::fs::read_to_string(&store.tasks_file).unwrap();
    let high_pos = content.find("high").unwrap();
    let mid_pos = content.find("mid").unwrap();
    let low_pos = content.find("low").unwrap();
    assert!(high_pos < mid_pos && mid_pos < low_pos);
}

#[test]
fn ideas_sort_active_before_done() {
    let dir = TempDir::new().unwrap();
    let store = IdeaStore::new(dir.path());

    let done = store.add("finished thing", 90, "olddone").unwrap();
    store.add("small", 10, "small").unwrap();
    store.mark_done(&done.id).unwrap();

    let ideas = store.load();
    assert_eq!(ideas[0].description, "small");
    assert!(ideas[1].is_done());
    assert_eq!(ideas[1].score, 90, "mark_done must preserve the score");
}

#[test]
fn mark_done_does_not_double_prefix() {
    let dir = TempDir::new().unwrap();
    let store = IdeaStore::new(dir.path());

    let idea = store.add("once only", 5, "once").unwrap();
    store.mark_done(&idea.id).unwrap();
    store.mark_done(&idea.id).unwrap();

    let reloaded = store.load();
    assert_eq!(reloaded[0].description, "(Done) once only");
}

#[test]
fn task_load_reads_both_todo_and_done_files() {
    let dir = TempDir::new().unwrap();
    let store = TaskStore::new(dir.path());

    let keep = store.add("keep", 5, "keep").unwrap();
    let finish = store.add("finish", 5, "finish").unwrap();
    store.update(&finish.id, &done_patch()).unwrap();

    let all = store.load();
    let ids: Vec<&str> = all.iter().map(|t| t.id.as_str()).collect();
    assert!(ids.contains(&keep.id.as_str()));
    assert!(ids.contains(&finish.id.as_str()));
}
