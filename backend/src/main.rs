mod artifacts;
mod auth;
mod docker;
mod extractor;
mod governance;
mod runtime;
mod servers;
mod telemetry;
use crate::routes::api_routes;
mod build;
mod capabilities;
mod config;
mod domains;
mod error;
mod evaluation;
mod evaluations;
mod file_store;
mod ingestion;
mod intelligence;
mod invocations;
mod job_queue;
mod marketplace;
mod organizations;
mod policy;
mod promotions;
mod proxy;
mod routes;
mod secrets;
mod services;
mod vault;
mod vector_dbs;
mod workflows;

use axum::{routing::get, Extension, Router};
use axum_prometheus::PrometheusMetricLayer;
use job_queue::start_worker;
use policy::{RuntimeBackend, RuntimePolicyEngine};
use runtime::{
    ContainerRuntime, DockerRuntime, InlineEvidenceAttestor, KubernetesRuntime, LocalVmProvisioner,
    RuntimeOrchestrator, VirtualMachineExecutor,
};
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing_subscriber::{fmt, EnvFilter};

async fn root() -> &'static str {
    "MCP Host API"
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .json()
        .init();

    dotenvy::dotenv().ok();
    // Fail fast if the JWT secret is missing
    let _ = config::JWT_SECRET.as_str();
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:password@localhost/mcp".into());
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await?;

    // Run migrations if available
    if let Err(error) = sqlx::migrate!().run(&pool).await {
        if *config::ALLOW_MIGRATION_FAILURE {
            tracing::warn!(
                ?error,
                "Database migrations failed but continuing due to ALLOW_MIGRATION_FAILURE"
            );
        } else {
            return Err(Box::new(error) as Box<dyn std::error::Error>);
        }
    }

    let configured_backend = config::CONTAINER_RUNTIME.as_str();
    let governance_engine = Arc::new(governance::GovernanceEngine::new());
    let mut policy_engine = Arc::new(RuntimePolicyEngine::new(match configured_backend {
        "kubernetes" => RuntimeBackend::Kubernetes,
        "virtual-machine" => RuntimeBackend::VirtualMachine,
        _ => RuntimeBackend::Docker,
    }));

    let runtime: Arc<dyn ContainerRuntime> = if configured_backend == "kubernetes" {
        policy_engine
            .register_executor(DockerRuntime::descriptor())
            .await;
        let docker_executor: Arc<dyn runtime::RuntimeExecutor> = Arc::new(DockerRuntime::new());

        match KubernetesRuntime::new().await {
            Ok(kube_runtime) => {
                policy_engine
                    .register_executor(KubernetesRuntime::descriptor())
                    .await;
                policy_engine
                    .attach_governance(governance_engine.clone())
                    .await;
                let executors: Vec<Arc<dyn runtime::RuntimeExecutor>> =
                    vec![docker_executor, Arc::new(kube_runtime)];
                Arc::new(RuntimeOrchestrator::new(
                    policy_engine.clone(),
                    pool.clone(),
                    executors,
                ))
            }
            Err(e) => {
                tracing::warn!(%e, "failed to init Kubernetes runtime; using docker");
                policy_engine = Arc::new(RuntimePolicyEngine::new(RuntimeBackend::Docker));
                policy_engine
                    .register_executor(DockerRuntime::descriptor())
                    .await;
                policy_engine
                    .attach_governance(governance_engine.clone())
                    .await;
                let executors: Vec<Arc<dyn runtime::RuntimeExecutor>> =
                    vec![Arc::new(DockerRuntime::new())];
                Arc::new(RuntimeOrchestrator::new(
                    policy_engine.clone(),
                    pool.clone(),
                    executors,
                ))
            }
        }
    } else if configured_backend == "virtual-machine" {
        policy_engine
            .register_executor(DockerRuntime::descriptor())
            .await;
        let docker_executor: Arc<dyn runtime::RuntimeExecutor> = Arc::new(DockerRuntime::new());
        let provisioner: Arc<dyn runtime::VmProvisioner> = Arc::new(LocalVmProvisioner);
        let attestor: Arc<dyn runtime::AttestationVerifier> = Arc::new(InlineEvidenceAttestor);
        let vm_executor: Arc<dyn runtime::RuntimeExecutor> = Arc::new(VirtualMachineExecutor::new(
            pool.clone(),
            provisioner,
            attestor,
        ));
        policy_engine
            .register_executor(VirtualMachineExecutor::descriptor())
            .await;
        policy_engine
            .attach_governance(governance_engine.clone())
            .await;
        let executors: Vec<Arc<dyn runtime::RuntimeExecutor>> = vec![docker_executor, vm_executor];
        Arc::new(RuntimeOrchestrator::new(
            policy_engine.clone(),
            pool.clone(),
            executors,
        ))
    } else {
        policy_engine
            .register_executor(DockerRuntime::descriptor())
            .await;
        policy_engine
            .attach_governance(governance_engine.clone())
            .await;
        let executors: Vec<Arc<dyn runtime::RuntimeExecutor>> =
            vec![Arc::new(DockerRuntime::new())];
        Arc::new(RuntimeOrchestrator::new(
            policy_engine.clone(),
            pool.clone(),
            executors,
        ))
    };
    let job_tx = start_worker(pool.clone(), runtime.clone());
    evaluations::scheduler::spawn(pool.clone(), job_tx.clone());
    ingestion::start_ingestion_worker(pool.clone());
    let (prometheus_layer, metrics_handle) = PrometheusMetricLayer::pair();
    let app = Router::new()
        .route("/", get(root))
        .route(
            "/metrics",
            get(move || async move { metrics_handle.render() }),
        )
        .merge(api_routes())
        .layer(prometheus_layer)
        .layer(Extension(pool.clone()))
        .layer(Extension(job_tx.clone()))
        .layer(Extension(runtime.clone()))
        .layer(Extension(policy_engine.clone()))
        .layer(Extension(governance_engine.clone()));

    let addr: SocketAddr = format!("{}:{}", config::BIND_ADDRESS.as_str(), *config::BIND_PORT)
        .parse()
        .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)?;
    tracing::info!(%addr, "Listening for incoming connections");
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}
