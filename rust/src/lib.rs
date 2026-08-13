//! pjpd — single-project task and idea management over MCP.
//!
//! The current working directory is the project root. All data lives under
//! `<cwd>/pjpd/` (tasks.txt, tasks_done.txt, ideas.txt) as plain text files.

pub mod ideas;
pub mod ids;
pub mod record;
pub mod server;
pub mod tasks;
pub mod textrec;
pub mod validation;
