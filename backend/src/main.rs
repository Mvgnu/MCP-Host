use axum::{routing::get, Extension, Router};
use axum_prometheus::PrometheusMetricLayer;
use base64::engine::general_purpose::STANDARD as Base64Engine;
use base64::Engine;
use backend::{
    config,
    evaluations,
    governance,
    ingestion,
    job_queue::start_worker,
    policy::{RuntimeBackend, RuntimePolicyEngine},
    remediation,
    routes::api_routes,
    runtime::{
        self,
        ContainerRuntime,
        DockerRuntime,
        HttpHypervisorProvisioner,
        KubernetesRuntime,
        RuntimeOrchestrator,
        TpmAttestationVerifier,
        VirtualMachineExecutor,
    },
    trust,
};
#[cfg(feature = "libvirt-executor")]
use backend::runtime::vm::libvirt::LibvirtVmProvisioner;
#[cfg(feature = "libvirt-executor")]
use backend::runtime::RealLibvirtDriver;
use ed25519_dalek::PublicKey;
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
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
        let provisioner: Arc<dyn runtime::VmProvisioner> = match *config::VM_PROVISIONER_DRIVER {
            config::VmProvisionerDriver::Http => Arc::new(HttpHypervisorProvisioner::new(
                config::VM_HYPERVISOR_ENDPOINT.clone(),
                (*config::VM_HYPERVISOR_TOKEN).clone(),
                *config::VM_LOG_TAIL_LINES,
            )?),
            config::VmProvisionerDriver::Libvirt => {
                #[cfg(feature = "libvirt-executor")]
                {
                    let libvirt_config = config::LIBVIRT_PROVISIONING_CONFIG.clone();
                    let driver: Arc<dyn runtime::LibvirtDriver> = Arc::new(RealLibvirtDriver::new(
                        libvirt_config.connection_uri.clone(),
                        libvirt_config.auth.clone(),
                        libvirt_config.console_source.clone(),
                    ));
                    Arc::new(LibvirtVmProvisioner::new(driver, libvirt_config))
                }
                #[cfg(not(feature = "libvirt-executor"))]
                {
                    panic!("libvirt executor requested but backend compiled without libvirt-executor feature");
                }
            }
        };
        let mut trust_roots = Vec::new();
        for encoded in config::VM_ATTESTATION_TRUST_ROOTS.iter() {
            match Base64Engine.decode(encoded) {
                Ok(bytes) if bytes.len() == 32 => {
                    let mut key_bytes = [0u8; 32];
                    key_bytes.copy_from_slice(&bytes);
                    match PublicKey::from_bytes(&key_bytes) {
                        Ok(key) => trust_roots.push(key),
                        Err(err) => tracing::warn!(?err, "failed to parse attestation trust root"),
                    }
                }
                Ok(_) => tracing::warn!("invalid trust root length"),
                Err(err) => tracing::warn!(?err, "failed to decode attestation trust root"),
            }
        }
        if trust_roots.is_empty() {
            tracing::warn!(
                "no attestation trust roots configured; relying on evidence-provided keys"
            );
        }
        let attestor: Arc<dyn runtime::AttestationVerifier> =
            Arc::new(TpmAttestationVerifier::new(
                (*config::VM_ATTESTATION_MEASUREMENTS).clone(),
                trust_roots,
                Duration::from_secs(*config::VM_ATTESTATION_MAX_AGE_SECONDS),
            ));
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
    trust::spawn_trust_listener(pool.clone(), job_tx.clone());
    remediation::spawn(pool.clone());
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
