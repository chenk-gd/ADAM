//! ADAM Server - Application entry point

use std::net::SocketAddr;
use std::sync::Arc;

use adam_adapters::mcp::{AdanMcpServer, McpServerState};
use adam_adapters::rest::{self, AppState};
use adam_domain::{AuthPrincipal, OrganizationId, ProjectId, Role};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    // Initialize repositories
    let asset_repo = Arc::new(adam_domain::InMemoryAssetRepository::new());
    let asset_type_repo = Arc::new(adam_domain::InMemoryAssetTypeRepository::new());
    let dependency_repo = Arc::new(adam_infrastructure::repositories::InMemoryDependencyRepository::new());
    let dirty_repo = Arc::new(adam_domain::InMemoryDirtyQueueRepository::new());
    let virtual_repo = Arc::new(adam_domain::InMemoryVirtualInstanceRepository::new());

    // Create a test principal (in production, this comes from auth token)
    let principal = AuthPrincipal {
        id: "server-default".to_string(),
        organization_id: OrganizationId::new(),
        project_memberships: vec![ProjectId::new()],
        roles: vec![Role::SystemAdmin],
    };

    // Create shared MCP server state
    let mcp_state = McpServerState {
        asset_repo: asset_repo.clone(),
        dependency_repo: dependency_repo.clone(),
        dirty_repo: dirty_repo.clone(),
        virtual_repo: virtual_repo.clone(),
        principal,
    };

    // Create MCP server
    let mcp_server = AdanMcpServer::new(mcp_state);

    // Start REST API server
    let rest_addr: SocketAddr = "0.0.0.0:3000".parse().expect("valid address");
    tracing::info!("ADAM REST API starting on {}", rest_addr);

    // Build REST router with AppState
    let app_state = AppState {
        asset_repo: asset_repo.clone(),
        asset_type_repo: asset_type_repo.clone(),
        dependency_repo: dependency_repo.clone(),
        dirty_repo: dirty_repo.clone(),
    };
    let rest_app = rest::create_router(app_state);

    // Start REST server in background
    let rest_handle = tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(rest_addr).await.unwrap();
        axum::serve(listener, rest_app).await.unwrap();
    });

    // Start MCP server (stdio transport for now)
    tracing::info!("ADAM MCP Server starting on stdio");
    tracing::info!("Use --mcp flag to start MCP server only");

    // Check command line args
    let args: Vec<String> = std::env::args().collect();
    if args.contains(&"--mcp".to_string()) {
        // Run MCP server only
        run_mcp_server(mcp_server).await?;
    } else if args.contains(&"--rest".to_string()) {
        // Run REST server only
        rest_handle.await?;
    } else {
        // Run both
        tokio::select! {
            result = rest_handle => result?,
            result = run_mcp_server(mcp_server) => result?,
        };
    }

    Ok(())
}

async fn run_mcp_server(server: AdanMcpServer) -> anyhow::Result<()> {
    use rmcp::service::ServiceExt;

    let service = server.serve(rmcp::transport::stdio()).await?;

    // Wait for cancellation
    service.cancel().await?;
    Ok(())
}
