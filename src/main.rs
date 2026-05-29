//! hackernews-mcp — a stdio-transport MCP server exposing read-only Hacker News
//! tools (official Firebase API + Algolia search) for use with Claude Desktop
//! and Claude Code.

mod client;
mod tools;
mod types;

use anyhow::Result;
use rmcp::{transport::stdio, ServiceExt};
use tracing_subscriber::EnvFilter;

use crate::tools::HackerNews;

#[tokio::main]
async fn main() -> Result<()> {
    // Logs go to stderr — stdout is reserved for the JSON-RPC stream.
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    tracing::info!("starting hackernews-mcp");

    let service = HackerNews::new()?.serve(stdio()).await.inspect_err(|e| {
        tracing::error!("failed to start server: {e:?}");
    })?;

    service.waiting().await?;
    Ok(())
}
