use once_cell::sync::Lazy;
use serde::Serialize;
use axum::{Json};

#[derive(Serialize, Clone)]
pub struct MarketplaceItem {
    pub server_type: String,
    pub image: String,
    pub description: String,
}

static ITEMS: Lazy<Vec<MarketplaceItem>> = Lazy::new(|| vec![
    MarketplaceItem { server_type: "PostgreSQL".into(), image: "ghcr.io/anycontext/postgres-mcp:latest".into(), description: "Expose a PostgreSQL database via MCP".into() },
    MarketplaceItem { server_type: "Slack".into(), image: "ghcr.io/anycontext/slack-mcp:latest".into(), description: "Query Slack channels".into() },
    MarketplaceItem { server_type: "PDF Parser".into(), image: "ghcr.io/anycontext/pdf-mcp:latest".into(), description: "Load and search PDF documents".into() },
    MarketplaceItem { server_type: "Notion".into(), image: "ghcr.io/anycontext/notion-mcp:latest".into(), description: "Connect to Notion pages".into() },
    MarketplaceItem { server_type: "Router".into(), image: "ghcr.io/anycontext/router-mcp:latest".into(), description: "Route queries to multiple MCPs".into() },
]);

pub async fn list_marketplace() -> Json<Vec<MarketplaceItem>> {
    Json(ITEMS.clone())
}
