//! ADAM Server - Application entry point

use std::net::SocketAddr;
use std::sync::Arc;

use adam_adapters::mcp::{AdamMcpServer, McpServerState};
use adam_adapters::rest::{self, AppState};
use adam_application::services::workflow::AgentTaskService;
use adam_domain::workflow::repository::{
    AgentTaskRepository, PromotionRuleRepository, WorkflowActionRepository,
    WorkflowEventRepository, WorkflowInstanceRepository,
};
use adam_domain::{
    AssetRepository, AssetTypeRepository, AssetVersionRepository, AuthPrincipal,
    DependencyRepository, DependencyRuleRepository, DirtyQueueRepository,
    DirtyResolutionLogRepository, OrganizationId, ProjectId, Role, VirtualInstanceRepository,
};
use sqlx::postgres::PgPoolOptions;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RepositoryBackend {
    Memory,
    Postgres,
}

impl RepositoryBackend {
    fn parse(value: &str) -> anyhow::Result<Self> {
        match value.to_ascii_lowercase().as_str() {
            "memory" => Ok(Self::Memory),
            "postgres" => Ok(Self::Postgres),
            other => anyhow::bail!(
                "Unsupported ADAM_REPOSITORY_BACKEND '{other}'. Use 'memory' or 'postgres'."
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunMode {
    Rest,
    Mcp,
    Both,
}

impl RunMode {
    fn from_args(args: &[String]) -> Self {
        if args.iter().any(|arg| arg == "--mcp") {
            Self::Mcp
        } else if args.iter().any(|arg| arg == "--both") {
            Self::Both
        } else {
            Self::Rest
        }
    }
}

#[derive(Debug, Clone)]
struct ServerConfig {
    backend: RepositoryBackend,
    database_url: Option<String>,
    rest_addr: SocketAddr,
}

impl ServerConfig {
    fn from_env() -> anyhow::Result<Self> {
        let backend = RepositoryBackend::parse(
            &std::env::var("ADAM_REPOSITORY_BACKEND").unwrap_or_else(|_| "memory".to_string()),
        )?;
        let host = std::env::var("ADAM_SERVER__HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port = std::env::var("ADAM_SERVER__PORT").unwrap_or_else(|_| "3000".to_string());
        let rest_addr = format!("{host}:{port}").parse()?;

        Ok(Self {
            backend,
            database_url: std::env::var("ADAM_DATABASE__URL").ok(),
            rest_addr,
        })
    }
}

#[derive(Clone)]
struct Repositories {
    asset_repo: Arc<dyn AssetRepository>,
    asset_type_repo: Arc<dyn AssetTypeRepository>,
    dependency_repo: Arc<dyn DependencyRepository>,
    dependency_rule_repo: Arc<dyn DependencyRuleRepository>,
    dirty_repo: Arc<dyn DirtyQueueRepository>,
    version_repo: Arc<dyn AssetVersionRepository>,
    dirty_log_repo: Arc<dyn DirtyResolutionLogRepository>,
    virtual_repo: Arc<dyn VirtualInstanceRepository>,
    // Workflow automation repositories (Slice 1)
    workflow_event_repo: Arc<dyn WorkflowEventRepository>,
    workflow_rule_repo: Arc<dyn PromotionRuleRepository>,
    workflow_instance_repo: Arc<dyn WorkflowInstanceRepository>,
    workflow_action_repo: Arc<dyn WorkflowActionRepository>,
    agent_task_repo: Arc<dyn AgentTaskRepository>,
}

impl Repositories {
    async fn from_config(config: &ServerConfig) -> anyhow::Result<Self> {
        match config.backend {
            RepositoryBackend::Memory => Ok(Self::memory()),
            RepositoryBackend::Postgres => {
                Self::postgres(config.database_url.as_deref().ok_or_else(|| {
                    anyhow::anyhow!("ADAM_DATABASE__URL is required for postgres")
                })?)
                .await
            }
        }
    }

    fn memory() -> Self {
        use adam_domain::workflow::in_memory::{
            InMemoryAgentTaskRepository, InMemoryPromotionRuleRepository,
            InMemoryWorkflowActionRepository, InMemoryWorkflowEventRepository,
            InMemoryWorkflowInstanceRepository,
        };
        Self {
            asset_repo: Arc::new(adam_domain::InMemoryAssetRepository::new()),
            asset_type_repo: Arc::new(adam_domain::InMemoryAssetTypeRepository::new()),
            dependency_repo: Arc::new(adam_domain::InMemoryDependencyRepository::new()),
            dependency_rule_repo: Arc::new(adam_domain::InMemoryDependencyRuleRepository::new()),
            dirty_repo: Arc::new(adam_domain::InMemoryDirtyQueueRepository::new()),
            version_repo: Arc::new(adam_domain::InMemoryAssetVersionRepository::new()),
            dirty_log_repo: Arc::new(adam_domain::InMemoryDirtyResolutionLogRepository::new()),
            virtual_repo: Arc::new(adam_domain::InMemoryVirtualInstanceRepository::new()),
            workflow_event_repo: Arc::new(InMemoryWorkflowEventRepository::default()),
            workflow_rule_repo: Arc::new(InMemoryPromotionRuleRepository::default()),
            workflow_instance_repo: Arc::new(InMemoryWorkflowInstanceRepository::default()),
            workflow_action_repo: Arc::new(InMemoryWorkflowActionRepository::default()),
            agent_task_repo: Arc::new(InMemoryAgentTaskRepository::default()),
        }
    }

    async fn postgres(database_url: &str) -> anyhow::Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await?;

        Ok(Self {
            asset_repo: Arc::new(
                adam_infrastructure::repositories::PostgresAssetRepository::new(pool.clone()),
            ),
            asset_type_repo: Arc::new(
                adam_infrastructure::repositories::PostgresAssetTypeRepository::new(pool.clone()),
            ),
            dependency_repo: Arc::new(
                adam_infrastructure::repositories::PostgresDependencyRepository::new(pool.clone()),
            ),
            dependency_rule_repo: Arc::new(
                adam_infrastructure::repositories::PostgresDependencyRuleRepository::new(
                    pool.clone(),
                ),
            ),
            dirty_repo: Arc::new(
                adam_infrastructure::repositories::PostgresDirtyQueueRepository::new(pool.clone()),
            ),
            version_repo: Arc::new(
                adam_infrastructure::repositories::PostgresAssetVersionRepository::new(
                    pool.clone(),
                ),
            ),
            dirty_log_repo: Arc::new(
                adam_infrastructure::repositories::PostgresDirtyResolutionLogRepository::new(
                    pool.clone(),
                ),
            ),
            virtual_repo: Arc::new(
                adam_infrastructure::repositories::PostgresVirtualInstanceRepository::new(
                    pool.clone(),
                ),
            ),
            workflow_event_repo: Arc::new(
                adam_infrastructure::repositories::PostgresWorkflowEventRepository::new(
                    pool.clone(),
                ),
            ),
            workflow_rule_repo: Arc::new(
                adam_infrastructure::repositories::PostgresPromotionRuleRepository::new(
                    pool.clone(),
                ),
            ),
            workflow_instance_repo: Arc::new(
                adam_infrastructure::repositories::PostgresWorkflowInstanceRepository::new(
                    pool.clone(),
                ),
            ),
            workflow_action_repo: Arc::new(
                adam_infrastructure::repositories::PostgresWorkflowActionRepository::new(
                    pool.clone(),
                ),
            ),
            agent_task_repo: Arc::new(
                adam_infrastructure::repositories::PostgresAgentTaskRepository::new(pool),
            ),
        })
    }

    fn rest_state(&self) -> AppState {
        AppState {
            asset_repo: self.asset_repo.clone(),
            asset_type_repo: self.asset_type_repo.clone(),
            dependency_repo: self.dependency_repo.clone(),
            dependency_rule_repo: self.dependency_rule_repo.clone(),
            dirty_repo: self.dirty_repo.clone(),
            version_repo: self.version_repo.clone(),
            dirty_log_repo: self.dirty_log_repo.clone(),
            workflow_event_repo: self.workflow_event_repo.clone(),
            workflow_rule_repo: self.workflow_rule_repo.clone(),
            workflow_instance_repo: self.workflow_instance_repo.clone(),
            workflow_action_repo: self.workflow_action_repo.clone(),
            agent_task_repo: self.agent_task_repo.clone(),
        }
    }

    fn mcp_state(&self, principal: AuthPrincipal) -> McpServerState {
        McpServerState {
            asset_repo: self.asset_repo.clone(),
            dependency_repo: self.dependency_repo.clone(),
            dependency_rule_repo: self.dependency_rule_repo.clone(),
            dirty_repo: self.dirty_repo.clone(),
            version_repo: self.version_repo.clone(),
            dirty_log_repo: self.dirty_log_repo.clone(),
            virtual_repo: self.virtual_repo.clone(),
            workflow_event_repo: self.workflow_event_repo.clone(),
            workflow_rule_repo: self.workflow_rule_repo.clone(),
            workflow_instance_repo: self.workflow_instance_repo.clone(),
            workflow_action_repo: self.workflow_action_repo.clone(),
            agent_task_repo: self.agent_task_repo.clone(),
            principal,
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let args: Vec<String> = std::env::args().collect();
    let mode = RunMode::from_args(&args);
    let config = ServerConfig::from_env()?;
    let repositories = Repositories::from_config(&config).await?;

    let principal = AuthPrincipal {
        id: "server-default".to_string(),
        organization_id: OrganizationId::new(),
        project_memberships: vec![ProjectId::new()],
        roles: vec![Role::SystemAdmin],
    };

    match mode {
        RunMode::Rest => run_rest_server(repositories.rest_state(), config.rest_addr).await?,
        RunMode::Mcp => {
            run_mcp_server(AdamMcpServer::new(repositories.mcp_state(principal))).await?
        }
        RunMode::Both => {
            let rest_state = repositories.rest_state();
            let rest_addr = config.rest_addr;
            let mcp_server = AdamMcpServer::new(repositories.mcp_state(principal));
            let rest_handle = tokio::spawn(run_rest_server(rest_state, rest_addr));

            tokio::select! {
                result = rest_handle => result??,
                result = run_mcp_server(mcp_server) => result?,
            };
        }
    }

    Ok(())
}

async fn run_rest_server(app_state: AppState, rest_addr: SocketAddr) -> anyhow::Result<()> {
    tracing::info!("ADAM REST API starting on {}", rest_addr);
    spawn_agent_task_expiry_worker(app_state.clone());
    let rest_app = rest::create_router(app_state);
    let listener = tokio::net::TcpListener::bind(rest_addr).await?;
    axum::serve(listener, rest_app).await?;
    Ok(())
}

fn spawn_agent_task_expiry_worker(app_state: AppState) {
    let interval_seconds = std::env::var("ADAM_AGENT_TASK_EXPIRY_INTERVAL_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(60);

    if interval_seconds == 0 {
        tracing::info!("Agent task expiry worker disabled");
        return;
    }

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_seconds));
        // If a scan overruns the interval, skip the missed ticks instead of
        // burst-firing a backlog of scans back-to-back.
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let service = AgentTaskService::new(
                app_state.agent_task_repo.clone(),
                app_state.workflow_action_repo.clone(),
                app_state.workflow_event_repo.clone(),
                app_state.workflow_instance_repo.clone(),
            );
            match service.timeout_expired(chrono::Utc::now()).await {
                Ok(expired) if !expired.is_empty() => {
                    tracing::info!(expired = expired.len(), "Expired agent tasks");
                }
                Ok(_) => {}
                Err(err) => tracing::warn!(error = %err, "Agent task expiry scan failed"),
            }
        }
    });
}

async fn run_mcp_server(server: AdamMcpServer) -> anyhow::Result<()> {
    use rmcp::service::ServiceExt;

    tracing::info!("ADAM MCP Server starting on stdio");
    let service = server.serve(rmcp::transport::stdio()).await?;
    service.cancel().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_backend_parses_supported_values() {
        assert_eq!(
            RepositoryBackend::parse("memory").unwrap(),
            RepositoryBackend::Memory
        );
        assert_eq!(
            RepositoryBackend::parse("postgres").unwrap(),
            RepositoryBackend::Postgres
        );
    }

    #[test]
    fn run_mode_defaults_to_rest() {
        let args = vec!["adam-server".to_string()];
        assert_eq!(RunMode::from_args(&args), RunMode::Rest);
    }

    #[test]
    fn run_mode_can_select_mcp_or_both() {
        let mcp_args = vec!["adam-server".to_string(), "--mcp".to_string()];
        let both_args = vec!["adam-server".to_string(), "--both".to_string()];

        assert_eq!(RunMode::from_args(&mcp_args), RunMode::Mcp);
        assert_eq!(RunMode::from_args(&both_args), RunMode::Both);
    }
}
