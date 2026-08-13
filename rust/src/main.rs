//! pjpd MCP server entry point. The current working directory is the project root.

use rmcp::{ServiceExt, transport::stdio};

use pjpd::server::PjpdServer;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("Starting Pjpd MCP Server...");
    let server = PjpdServer::new(std::env::current_dir()?);
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
