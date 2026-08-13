//! MCP server surface: tool and prompt definitions.
//!
//! Every tool returns `{"success": bool, "result": ..., "error": "..."}` as
//! JSON text content. Every successful response includes a `project_file`
//! property with the full path to the relevant data file.

use std::path::{Path, PathBuf};

use rmcp::handler::server::router::{prompt::PromptRouter, tool::ToolRouter};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ContentBlock, Implementation, PromptMessage, Role, ServerCapabilities,
    ServerInfo,
};
use rmcp::{ServerHandler, prompt, prompt_handler, prompt_router, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::ideas::IdeaStore;
use crate::tasks::{Task, TaskPatch, TaskStore};
use crate::validation;

const INTRO_TEXT: &str = include_str!("../../resources/intro.txt");

/// Priority assigned to newly created tasks when the caller omits one.
const DEFAULT_NEW_TASK_PRIORITY: i64 = 50;

fn default_count() -> i64 {
    20
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct PutTaskParams {
    /// What needs to be done, in enough detail to act on without additional
    /// context. Self-contained: do not reference other task IDs.
    pub description: String,
    /// Tag string (1-12 characters, alphanumeric and hyphens only). Provide to
    /// create a new task. Must describe the task itself (e.g. "oplock",
    /// "gitsemantics"), NEVER the project name — tasks already live inside a
    /// project, and the tag makes the task ID self-describing.
    #[serde(default)]
    pub tag: Option<String>,
    /// Existing task ID (format: `<tag>-XXXX`). Provide to update an existing task.
    #[serde(default)]
    pub id: Option<String>,
    /// Priority from 0 (negligible) to 100 (urgent). Values outside this range
    /// are allowed for exceptional cases. Defaults to 50 when creating a new
    /// task. When updating, omit to preserve the task's existing priority.
    #[serde(default)]
    pub priority: Option<i64>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ListTasksParams {
    /// Maximum number of tasks to return. Defaults to 20.
    #[serde(default = "default_count")]
    pub count: i64,
    /// Whether to include completed tasks. Defaults to False.
    #[serde(default)]
    pub show_done: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct MarkTaskDoneParams {
    /// List of tag-based task IDs (format: `<tag>-XXXX`) to mark as done.
    pub task_ids: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ReprioritizeTaskParams {
    /// New priority to apply to every listed task. 0 (negligible) to 100
    /// (urgent); values outside this range are allowed for exceptional cases.
    pub priority: i64,
    /// List of tag-based task IDs (format: `<tag>-XXXX`) to reprioritize.
    pub task_ids: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ListIdeasParams {
    /// Maximum number of ideas to return. Defaults to 20.
    #[serde(default = "default_count")]
    pub count: i64,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct PutIdeaParams {
    /// Relevance score from 0 (trivial) to 100 (critical). Values outside this
    /// range are allowed for exceptional cases.
    pub score: i64,
    /// What the idea entails, in enough detail for a human or model to
    /// understand its purpose and scope.
    pub description: String,
    /// Tag string (1-12 characters, alphanumeric and hyphens only). Provide to
    /// create a new idea. Must describe the idea itself, NEVER the project
    /// name — ideas already live inside a project, and the tag makes the idea
    /// ID self-describing.
    #[serde(default)]
    pub tag: Option<String>,
    /// Existing idea ID (format: `<tag>-XXXX`). Provide to update an existing idea.
    #[serde(default)]
    pub id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct MarkIdeaDoneParams {
    /// List of tag-based idea IDs (format: `<tag>-XXXX`) to mark as done.
    pub idea_ids: Vec<String>,
}

fn mcp_success(result: Value) -> CallToolResult {
    to_tool_result(json!({"success": true, "result": result, "error": ""}))
}

fn mcp_failure(error_message: &str) -> CallToolResult {
    to_tool_result(json!({"success": false, "result": "", "error": error_message}))
}

fn to_tool_result(response: Value) -> CallToolResult {
    let text = serde_json::to_string_pretty(&response).unwrap_or_else(|e| {
        format!("{{\"success\": false, \"result\": \"\", \"error\": \"serialization error: {e}\"}}")
    });
    CallToolResult::success(vec![ContentBlock::text(text)])
}

/// Render a string list the way Python's str(list) does: ['a', 'b'].
fn python_list_repr(items: &[String]) -> String {
    let quoted: Vec<String> = items.iter().map(|s| format!("'{s}'")).collect();
    format!("[{}]", quoted.join(", "))
}

pub struct PjpdServer {
    project_dir: PathBuf,
    tool_router: ToolRouter<Self>,
    prompt_router: PromptRouter<Self>,
}

impl PjpdServer {
    pub fn new(project_dir: PathBuf) -> Self {
        Self {
            project_dir,
            tool_router: Self::tool_router(),
            prompt_router: Self::prompt_router(),
        }
    }

    fn tasks(&self) -> TaskStore {
        TaskStore::new(&self.project_dir)
    }

    fn ideas(&self) -> IdeaStore {
        IdeaStore::new(&self.project_dir)
    }

    /// Add the legacy project file warning to a result map if applicable.
    fn with_warning(&self, mut result: Map<String, Value>) -> Map<String, Value> {
        if let Some(warning) = self.tasks().legacy_warning() {
            result.insert("warning".to_string(), json!(warning));
        }
        result
    }

    fn task_response(&self, task: &Task, message: &str) -> CallToolResult {
        let result = record_map(task.to_json(), &self.tasks().tasks_file, message);
        mcp_success(Value::Object(self.with_warning(result)))
    }

    fn put_task_impl(&self, params: PutTaskParams) -> Result<CallToolResult, String> {
        validation::validate_non_empty_description(&params.description)?;
        if let Some(priority) = params.priority {
            validation::validate_range(priority, 0, 9999, "priority")?;
        }
        validation::validate_tag_or_id(&params.tag, &params.id, "Task ID")?;

        if let Some(tag) = &params.tag {
            let priority = params.priority.unwrap_or(DEFAULT_NEW_TASK_PRIORITY);
            let task = self.tasks().add(&params.description, priority, tag)?;
            return Ok(self.task_response(&task, "Task created successfully"));
        }

        let id = params.id.as_deref().unwrap_or_default();
        let patch = TaskPatch {
            description: Some(params.description.clone()),
            priority: params.priority,
            status: None,
        };
        match self.tasks().update(id, &patch)? {
            Some(task) => {
                Ok(self.task_response(&task, &format!("Task '{id}' updated successfully")))
            }
            None => Ok(mcp_failure(&format!(
                "Task '{id}' not found. Use pjpd_list_tasks() to see existing task IDs."
            ))),
        }
    }

    fn list_tasks_impl(&self, params: ListTasksParams) -> Result<CallToolResult, String> {
        validation::validate_range(params.count, 1, 1000, "count")?;

        let mut tasks = self.tasks().load();
        if !params.show_done {
            tasks.retain(|t| t.status.eq_ignore_ascii_case("todo"));
        }

        // Sort: ToDo before Done, then priority desc, then description.
        let status_rank = |status: &str| match status {
            "ToDo" => 0,
            "Done" => 1,
            _ => 2,
        };
        tasks.sort_by_key(|t| {
            (
                status_rank(&t.status),
                std::cmp::Reverse(t.priority),
                t.description.to_lowercase(),
            )
        });

        let total = tasks.len();
        tasks.truncate(params.count as usize);

        let task_dicts: Vec<Value> = tasks
            .iter()
            .map(|t| {
                json!({
                    "id": t.id,
                    "priority": t.priority,
                    "status": t.status,
                    "description": t.description,
                })
            })
            .collect();

        let mut result = Map::new();
        result.insert("total_tasks".to_string(), json!(tasks.len()));
        result.insert("tasks".to_string(), json!(task_dicts));
        result.insert(
            "project_file".to_string(),
            json!(self.tasks().tasks_file.to_string_lossy()),
        );
        let mut result = self.with_warning(result);
        if total > params.count as usize {
            result.insert(
                "info".to_string(),
                json!(format!("Returned {} of {} total tasks", tasks.len(), total)),
            );
        }
        Ok(mcp_success(Value::Object(result)))
    }

    /// All-or-nothing bulk task update: if any ID is missing, nothing is modified.
    fn bulk_update_tasks(
        &self,
        task_ids: &[String],
        patch: &TaskPatch,
        message: &str,
    ) -> Result<CallToolResult, String> {
        let store = self.tasks();
        let missing: Vec<String> = task_ids
            .iter()
            .filter(|id| store.get(id).is_none())
            .cloned()
            .collect();
        if !missing.is_empty() {
            return Ok(mcp_failure(&format!(
                "Tasks not found: {}. No tasks were modified. \
                 Use pjpd_list_tasks(show_done=True) to see all task IDs.",
                python_list_repr(&missing)
            )));
        }

        let updated = task_ids
            .iter()
            .map(|id| {
                store
                    .update(id, patch)?
                    .map(|t| t.to_json())
                    .ok_or_else(|| format!("Task '{id}' disappeared during update"))
            })
            .collect::<Result<Vec<Value>, String>>()?;

        let mut result = Map::new();
        result.insert("tasks".to_string(), json!(updated));
        result.insert(
            "project_file".to_string(),
            json!(store.tasks_file.to_string_lossy()),
        );
        result.insert("message".to_string(), json!(message));
        Ok(mcp_success(Value::Object(self.with_warning(result))))
    }

    fn list_ideas_impl(&self, params: ListIdeasParams) -> Result<CallToolResult, String> {
        validation::validate_range(params.count, 1, 1000, "count")?;

        let mut ideas = self.ideas().load();
        ideas.truncate(params.count as usize);
        let idea_dicts: Vec<Value> = ideas.iter().map(|i| i.to_json()).collect();

        Ok(mcp_success(json!({
            "total_ideas": idea_dicts.len(),
            "ideas": idea_dicts,
            "project_file": self.ideas().ideas_file.to_string_lossy(),
            "message": format!("Retrieved {} ideas", idea_dicts.len()),
        })))
    }

    fn put_idea_impl(&self, params: PutIdeaParams) -> Result<CallToolResult, String> {
        validation::validate_range(params.score, 0, 9999, "score")?;
        validation::validate_non_empty_description(&params.description)?;
        validation::validate_tag_or_id(&params.tag, &params.id, "Idea ID")?;

        let store = self.ideas();
        if let Some(tag) = &params.tag {
            let idea = store.add(&params.description, params.score, tag)?;
            return Ok(idea_response(
                &store,
                &idea,
                &format!("Idea created successfully with ID '{}'", idea.id),
            ));
        }

        let id = params.id.as_deref().unwrap_or_default();
        match store.update(id, &params.description, params.score)? {
            Some(idea) => Ok(idea_response(
                &store,
                &idea,
                &format!("Idea '{id}' updated successfully"),
            )),
            None => Ok(mcp_failure(&format!(
                "Idea '{id}' not found. Use pjpd_list_ideas() to see existing idea IDs."
            ))),
        }
    }

    fn mark_idea_done_impl(&self, params: MarkIdeaDoneParams) -> Result<CallToolResult, String> {
        validation::validate_id_list(&params.idea_ids, "Idea ID")?;

        let store = self.ideas();
        let known: std::collections::HashSet<String> =
            store.load().into_iter().map(|i| i.id).collect();
        let missing: Vec<String> = params
            .idea_ids
            .iter()
            .filter(|id| !known.contains(*id))
            .cloned()
            .collect();
        if !missing.is_empty() {
            return Ok(mcp_failure(&format!(
                "Ideas not found: {}. No ideas were modified. \
                 Use pjpd_list_ideas() to see existing idea IDs.",
                python_list_repr(&missing)
            )));
        }

        for id in &params.idea_ids {
            store.mark_done(id)?;
        }

        Ok(mcp_success(json!({
            "idea_ids": params.idea_ids,
            "project_file": store.ideas_file.to_string_lossy(),
            "message": format!("Marked {} idea(s) as done", params.idea_ids.len()),
        })))
    }
}

fn idea_response(store: &IdeaStore, idea: &crate::ideas::Idea, message: &str) -> CallToolResult {
    mcp_success(Value::Object(record_map(
        idea.to_json(),
        &store.ideas_file,
        message,
    )))
}

/// The common response body for a single record: its fields plus
/// `project_file` and `message`.
fn record_map(record_json: Value, project_file: &Path, message: &str) -> Map<String, Value> {
    let mut map = match record_json {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    map.insert(
        "project_file".to_string(),
        json!(project_file.to_string_lossy()),
    );
    map.insert("message".to_string(), json!(message));
    map
}

#[tool_router]
impl PjpdServer {
    #[tool(
        name = "pjpd_put_task",
        description = "Create or update a task. Provide `tag` to create a new task, or `id` to update an existing one."
    )]
    pub async fn pjpd_put_task(
        &self,
        Parameters(params): Parameters<PutTaskParams>,
    ) -> CallToolResult {
        self.put_task_impl(params)
            .unwrap_or_else(|e| mcp_failure(&format!("Error putting task: {e}")))
    }

    #[tool(
        name = "pjpd_list_tasks",
        description = "List tasks with optional filtering. Tasks are sorted ToDo before Done, then by priority descending."
    )]
    pub async fn pjpd_list_tasks(
        &self,
        Parameters(params): Parameters<ListTasksParams>,
    ) -> CallToolResult {
        self.list_tasks_impl(params)
            .unwrap_or_else(|e| mcp_failure(&format!("Error listing tasks: {e}")))
    }

    #[tool(
        name = "pjpd_mark_task_done",
        description = "Mark one or more tasks as completed. All IDs must exist or nothing is changed."
    )]
    pub async fn pjpd_mark_task_done(
        &self,
        Parameters(params): Parameters<MarkTaskDoneParams>,
    ) -> CallToolResult {
        let run = || -> Result<CallToolResult, String> {
            validation::validate_id_list(&params.task_ids, "Task ID")?;
            let done = TaskPatch {
                description: None,
                priority: None,
                status: Some(crate::tasks::STATUS_DONE.to_string()),
            };
            self.bulk_update_tasks(
                &params.task_ids,
                &done,
                &format!("Marked {} task(s) as done", params.task_ids.len()),
            )
        };
        run().unwrap_or_else(|e| mcp_failure(&format!("Error marking tasks as done: {e}")))
    }

    #[tool(
        name = "pjpd_reprioritize_task",
        description = "Set the priority of one or more tasks. All IDs must exist or nothing is changed. Use this to shuffle priorities without resending the description text."
    )]
    pub async fn pjpd_reprioritize_task(
        &self,
        Parameters(params): Parameters<ReprioritizeTaskParams>,
    ) -> CallToolResult {
        let run = || -> Result<CallToolResult, String> {
            validation::validate_range(params.priority, 0, 9999, "priority")?;
            validation::validate_id_list(&params.task_ids, "Task ID")?;
            let reprioritize = TaskPatch {
                description: None,
                priority: Some(params.priority),
                status: None,
            };
            self.bulk_update_tasks(
                &params.task_ids,
                &reprioritize,
                &format!(
                    "Reprioritized {} task(s) to priority {}",
                    params.task_ids.len(),
                    params.priority
                ),
            )
        };
        run().unwrap_or_else(|e| {
            mcp_failure(&format!(
                "Error reprioritizing tasks: {e}. \
                 Possible fixes: 1) Ensure each task_id matches <tag>-XXXX format (e.g. 'task-ab12'), \
                 2) Pass priority as an integer (0-100 typical, higher allowed), \
                 3) Use pjpd_list_tasks(show_done=True) to confirm the task IDs exist."
            ))
        })
    }

    #[tool(
        name = "pjpd_list_ideas",
        description = "List ideas sorted by score (highest first)."
    )]
    pub async fn pjpd_list_ideas(
        &self,
        Parameters(params): Parameters<ListIdeasParams>,
    ) -> CallToolResult {
        self.list_ideas_impl(params)
            .unwrap_or_else(|e| mcp_failure(&format!("Error listing ideas: {e}")))
    }

    #[tool(
        name = "pjpd_put_idea",
        description = "Create or update an idea. Provide `tag` to create a new idea, or `id` to update an existing one."
    )]
    pub async fn pjpd_put_idea(
        &self,
        Parameters(params): Parameters<PutIdeaParams>,
    ) -> CallToolResult {
        self.put_idea_impl(params)
            .unwrap_or_else(|e| mcp_failure(&format!("Error putting idea: {e}")))
    }

    #[tool(
        name = "pjpd_mark_idea_done",
        description = "Mark one or more ideas as done (description prefixed with '(Done)'; the score is preserved so it stays visible even though the idea is off the books). All IDs must exist or nothing is changed."
    )]
    pub async fn pjpd_mark_idea_done(
        &self,
        Parameters(params): Parameters<MarkIdeaDoneParams>,
    ) -> CallToolResult {
        self.mark_idea_done_impl(params)
            .unwrap_or_else(|e| mcp_failure(&format!("Error marking ideas as done: {e}")))
    }
}

#[prompt_router]
impl PjpdServer {
    #[prompt(
        name = "pjpd_intro",
        description = "Return an introductory prompt that describes the ProjectMCP system."
    )]
    pub async fn pjpd_intro(&self) -> Vec<PromptMessage> {
        vec![PromptMessage::new_text(Role::User, INTRO_TEXT)]
    }
}

#[tool_handler(router = self.tool_router)]
#[prompt_handler(router = self.prompt_router)]
impl ServerHandler for PjpdServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_prompts()
                .build(),
        )
        .with_server_info(Implementation::new("projectmcp", env!("CARGO_PKG_VERSION")))
    }
}
