//! Tool-level tests exercising the full request -> disk -> response path,
//! mirroring the behaviors covered by the Python test suite.

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use serde_json::Value;
use tempfile::TempDir;

use pjpd::server::{
    ListIdeasParams, ListTasksParams, MarkIdeaDoneParams, MarkTaskDoneParams, PjpdServer,
    PutIdeaParams, PutTaskParams, ReprioritizeTaskParams,
};

fn server() -> (TempDir, PjpdServer) {
    let dir = TempDir::new().unwrap();
    let server = PjpdServer::new(dir.path().to_path_buf());
    (dir, server)
}

fn response_json(result: &CallToolResult) -> Value {
    let text = &result.content[0].as_text().expect("text content").text;
    serde_json::from_str(text).expect("valid JSON response")
}

fn assert_success(response: &Value) -> Value {
    assert_eq!(
        response["success"],
        Value::Bool(true),
        "expected success, got: {response}"
    );
    response["result"].clone()
}

fn assert_failure(response: &Value) -> String {
    assert_eq!(
        response["success"],
        Value::Bool(false),
        "expected failure, got: {response}"
    );
    response["error"].as_str().unwrap().to_string()
}

async fn create_task(server: &PjpdServer, description: &str, tag: &str, priority: i64) -> String {
    let result = server
        .pjpd_put_task(Parameters(PutTaskParams {
            description: description.to_string(),
            tag: Some(tag.to_string()),
            id: None,
            priority: Some(priority),
        }))
        .await;
    let body = assert_success(&response_json(&result));
    body["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn put_task_creates_task_with_metadata() {
    let (dir, server) = server();
    let result = server
        .pjpd_put_task(Parameters(PutTaskParams {
            description: "Write the docs".to_string(),
            tag: Some("docs".to_string()),
            id: None,
            priority: Some(7),
        }))
        .await;
    let body = assert_success(&response_json(&result));

    assert!(body["id"].as_str().unwrap().starts_with("docs-"));
    assert_eq!(body["priority"], 7);
    assert_eq!(body["status"], "ToDo");
    assert_eq!(body["message"], "Task created successfully");
    assert!(body["created"].as_str().unwrap().ends_with('Z'));
    assert!(
        body["project_file"]
            .as_str()
            .unwrap()
            .ends_with("pjpd/tasks.txt")
    );
    assert!(dir.path().join("pjpd/tasks.txt").exists());
}

#[tokio::test]
async fn put_task_updates_existing_task() {
    let (_dir, server) = server();
    let id = create_task(&server, "original", "fix", 3).await;

    let result = server
        .pjpd_put_task(Parameters(PutTaskParams {
            description: "updated text".to_string(),
            tag: None,
            id: Some(id.clone()),
            priority: Some(9),
        }))
        .await;
    let body = assert_success(&response_json(&result));
    assert_eq!(body["description"], "updated text");
    assert_eq!(body["priority"], 9);
    assert_eq!(body["message"], format!("Task '{id}' updated successfully"));
}

#[tokio::test]
async fn put_task_update_without_priority_preserves_existing() {
    let (_dir, server) = server();
    let id = create_task(&server, "keep my priority", "keep", 33).await;

    let result = server
        .pjpd_put_task(Parameters(PutTaskParams {
            description: "new words, same priority".to_string(),
            tag: None,
            id: Some(id.clone()),
            priority: None,
        }))
        .await;
    let body = assert_success(&response_json(&result));
    assert_eq!(body["description"], "new words, same priority");
    assert_eq!(body["priority"], 33);
}

#[tokio::test]
async fn put_task_create_without_priority_defaults_to_50() {
    let (_dir, server) = server();
    let result = server
        .pjpd_put_task(Parameters(PutTaskParams {
            description: "default priority".to_string(),
            tag: Some("defp".to_string()),
            id: None,
            priority: None,
        }))
        .await;
    let body = assert_success(&response_json(&result));
    assert_eq!(body["priority"], 50);
}

#[tokio::test]
async fn put_task_rejects_tag_and_id_together() {
    let (_dir, server) = server();
    let result = server
        .pjpd_put_task(Parameters(PutTaskParams {
            description: "x".to_string(),
            tag: Some("t".to_string()),
            id: Some("t-ab29".to_string()),
            priority: Some(50),
        }))
        .await;
    let error = assert_failure(&response_json(&result));
    assert!(error.contains("not both"), "got: {error}");
}

#[tokio::test]
async fn put_task_unknown_id_fails_gracefully() {
    let (_dir, server) = server();
    let result = server
        .pjpd_put_task(Parameters(PutTaskParams {
            description: "x".to_string(),
            tag: None,
            id: Some("ghost-ab29".to_string()),
            priority: Some(50),
        }))
        .await;
    let error = assert_failure(&response_json(&result));
    assert!(
        error.contains("Task 'ghost-ab29' not found"),
        "got: {error}"
    );
}

#[tokio::test]
async fn list_tasks_sorts_and_truncates() {
    let (_dir, server) = server();
    create_task(&server, "low", "low", 1).await;
    create_task(&server, "high", "high", 90).await;
    create_task(&server, "mid", "mid", 40).await;

    let result = server
        .pjpd_list_tasks(Parameters(ListTasksParams {
            count: 2,
            show_done: false,
        }))
        .await;
    let body = assert_success(&response_json(&result));
    assert_eq!(body["total_tasks"], 2);
    let descriptions: Vec<&str> = body["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["description"].as_str().unwrap())
        .collect();
    assert_eq!(descriptions, vec!["high", "mid"]);
    assert_eq!(body["info"], "Returned 2 of 3 total tasks");
}

#[tokio::test]
async fn mark_task_done_moves_record_to_done_file() {
    let (dir, server) = server();
    let id = create_task(&server, "finish me", "done", 5).await;

    let result = server
        .pjpd_mark_task_done(Parameters(MarkTaskDoneParams {
            task_ids: vec![id.clone()],
        }))
        .await;
    let body = assert_success(&response_json(&result));
    assert_eq!(body["message"], "Marked 1 task(s) as done");
    assert_eq!(body["tasks"][0]["status"], "Done");
    assert_eq!(body["tasks"][0]["priority"], 5);

    let todo = std::fs::read_to_string(dir.path().join("pjpd/tasks.txt")).unwrap();
    let done = std::fs::read_to_string(dir.path().join("pjpd/tasks_done.txt")).unwrap();
    assert!(!todo.contains(&id));
    assert!(done.contains(&id));

    // Done tasks are hidden by default, visible with show_done.
    let hidden = assert_success(&response_json(
        &server
            .pjpd_list_tasks(Parameters(ListTasksParams {
                count: 20,
                show_done: false,
            }))
            .await,
    ));
    assert_eq!(hidden["total_tasks"], 0);
    let shown = assert_success(&response_json(
        &server
            .pjpd_list_tasks(Parameters(ListTasksParams {
                count: 20,
                show_done: true,
            }))
            .await,
    ));
    assert_eq!(shown["total_tasks"], 1);
}

#[tokio::test]
async fn bulk_operations_are_all_or_nothing() {
    let (_dir, server) = server();
    let real = create_task(&server, "real", "real", 5).await;

    let result = server
        .pjpd_mark_task_done(Parameters(MarkTaskDoneParams {
            task_ids: vec![real.clone(), "ghost-ab29".to_string()],
        }))
        .await;
    let error = assert_failure(&response_json(&result));
    assert!(
        error.contains("Tasks not found: ['ghost-ab29']"),
        "got: {error}"
    );
    assert!(error.contains("No tasks were modified"));

    // The real task must still be ToDo.
    let body = assert_success(&response_json(
        &server
            .pjpd_list_tasks(Parameters(ListTasksParams {
                count: 20,
                show_done: false,
            }))
            .await,
    ));
    assert_eq!(body["total_tasks"], 1);
}

#[tokio::test]
async fn reprioritize_updates_priority_only() {
    let (_dir, server) = server();
    let id = create_task(&server, "shuffle me", "prio", 5).await;

    let result = server
        .pjpd_reprioritize_task(Parameters(ReprioritizeTaskParams {
            priority: 77,
            task_ids: vec![id.clone()],
        }))
        .await;
    let body = assert_success(&response_json(&result));
    assert_eq!(body["message"], "Reprioritized 1 task(s) to priority 77");
    assert_eq!(body["tasks"][0]["priority"], 77);
    assert_eq!(body["tasks"][0]["description"], "shuffle me");
}

#[tokio::test]
async fn put_idea_and_list_ideas_roundtrip() {
    let (_dir, server) = server();
    let result = server
        .pjpd_put_idea(Parameters(PutIdeaParams {
            score: 60,
            description: "brilliant plan".to_string(),
            tag: Some("plan".to_string()),
            id: None,
        }))
        .await;
    let body = assert_success(&response_json(&result));
    let id = body["id"].as_str().unwrap().to_string();
    assert!(id.starts_with("plan-"));
    assert_eq!(
        body["message"],
        format!("Idea created successfully with ID '{id}'")
    );

    let listed = assert_success(&response_json(
        &server
            .pjpd_list_ideas(Parameters(ListIdeasParams { count: 20 }))
            .await,
    ));
    assert_eq!(listed["total_ideas"], 1);
    assert_eq!(listed["ideas"][0]["score"], 60);
    assert_eq!(listed["message"], "Retrieved 1 ideas");
}

#[tokio::test]
async fn mark_idea_done_prefixes_description_and_preserves_score() {
    let (dir, server) = server();
    let created = assert_success(&response_json(
        &server
            .pjpd_put_idea(Parameters(PutIdeaParams {
                score: 42,
                description: "keep my score".to_string(),
                tag: Some("keep".to_string()),
                id: None,
            }))
            .await,
    ));
    let id = created["id"].as_str().unwrap().to_string();

    let result = server
        .pjpd_mark_idea_done(Parameters(MarkIdeaDoneParams {
            idea_ids: vec![id.clone()],
        }))
        .await;
    let body = assert_success(&response_json(&result));
    assert_eq!(body["message"], "Marked 1 idea(s) as done");

    let listed = assert_success(&response_json(
        &server
            .pjpd_list_ideas(Parameters(ListIdeasParams { count: 20 }))
            .await,
    ));
    assert_eq!(listed["ideas"][0]["description"], "(Done) keep my score");
    assert_eq!(listed["ideas"][0]["score"], 42);

    let on_disk = std::fs::read_to_string(dir.path().join("pjpd/ideas.txt")).unwrap();
    assert!(on_disk.contains("Score:   42"));
    assert!(on_disk.contains("(Done) keep my score"));
}

#[tokio::test]
async fn tasks_file_backup_lands_in_bak_directory() {
    let (dir, server) = server();
    create_task(&server, "first", "one", 5).await;
    create_task(&server, "second", "two", 6).await;

    let bak_dir = dir.path().join("pjpd/bak");
    assert!(
        bak_dir.exists(),
        "bak/ directory should exist after rewrite"
    );
    let backups: Vec<_> = std::fs::read_dir(&bak_dir).unwrap().collect();
    assert!(!backups.is_empty(), "expected at least one backup file");
}

#[tokio::test]
async fn legacy_project_file_produces_warning() {
    let (dir, server) = server();
    let dir_name = dir
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    std::fs::create_dir_all(dir.path().join("pjpd")).unwrap();
    std::fs::write(
        dir.path().join("pjpd").join(format!("{dir_name}.txt")),
        "old stuff",
    )
    .unwrap();

    create_task(&server, "new task", "new", 5).await;
    let result = server
        .pjpd_list_tasks(Parameters(ListTasksParams {
            count: 20,
            show_done: false,
        }))
        .await;
    let body = assert_success(&response_json(&result));
    assert!(
        body["warning"]
            .as_str()
            .unwrap()
            .contains("Legacy project file")
    );
}
