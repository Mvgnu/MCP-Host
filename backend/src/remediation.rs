use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{json, Value};
use sqlx::{PgPool, Postgres, Transaction};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

use crate::db::runtime_vm_remediation_artifacts::insert_artifact;
use crate::db::runtime_vm_remediation_playbooks::{
    get_by_id as get_playbook_by_id, get_by_key as get_playbook_by_key,
    RuntimeVmRemediationPlaybook,
};
use crate::db::runtime_vm_remediation_runs::{
    ensure_remediation_run, get_active_run_for_instance, mark_run_completed, mark_run_failed,
    try_acquire_next_run, EnsureRemediationRunRequest, RuntimeVmRemediationRun,
};
use crate::db::runtime_vm_trust_registry::{
    get_state as get_registry_state, upsert_state as upsert_registry_state,
    UpsertRuntimeVmTrustRegistryState,
};
use crate::trust::{subscribe_registry_events, TrustRegistryEvent};

const DEFAULT_PLAYBOOK: &str = "default-vm-remediation";
const REMEDIATION_STREAM_BUFFER: usize = 64;

// key: remediation-orchestrator -> execution-engine
pub fn spawn(pool: PgPool) {
    let registry = Arc::new(RemediationExecutorRegistry::bootstrap());
    let pool_clone = pool.clone();
    let registry_clone = registry.clone();

    tokio::spawn(async move {
        remediation_event_listener(pool_clone, registry_clone).await;
    });

    tokio::spawn(async move {
        remediation_worker(pool, registry).await;
    });
}

async fn remediation_event_listener(pool: PgPool, registry: Arc<RemediationExecutorRegistry>) {
    let mut receiver = subscribe_registry_events();
    while let Ok(event) = receiver.recv().await {
        if event.lifecycle_state == "quarantined" {
            if let Err(err) = handle_quarantine_event(&pool, &registry, &event).await {
                error!(
                    ?err,
                    vm_instance_id = event.vm_instance_id,
                    "failed to stage remediation run for quarantined instance",
                );
            }
        }
    }
    warn!("remediation orchestrator receiver dropped; remediation listener exiting");
}

async fn remediation_worker(pool: PgPool, registry: Arc<RemediationExecutorRegistry>) {
    loop {
        match dispatch_next_run(&pool, &registry).await {
            Ok(Some(_)) => {
                continue;
            }
            Ok(None) => {
                sleep(Duration::from_secs(1)).await;
            }
            Err(err) => {
                error!(?err, "remediation worker failed to dispatch next run");
                sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

async fn dispatch_next_run(
    pool: &PgPool,
    registry: &Arc<RemediationExecutorRegistry>,
) -> Result<Option<()>, RemediationError> {
    let mut tx = pool.begin().await?;
    let Some(run) = try_acquire_next_run(&mut *tx).await? else {
        tx.rollback().await?;
        return Ok(None);
    };

    let playbook = resolve_playbook(pool, &run).await?;
    let registry_state = get_registry_state(&mut *tx, run.runtime_vm_instance_id).await?;
    let (attestation_status, remediation_attempts, freshness_deadline, expected_version) =
        if let Some(state) = registry_state.as_ref() {
            (
                state.attestation_status.as_str(),
                state.remediation_attempts + 1,
                state.freshness_deadline,
                Some(state.version),
            )
        } else {
            ("unknown", 1, None, None)
        };

    upsert_registry_state(
        &mut *tx,
        UpsertRuntimeVmTrustRegistryState {
            runtime_vm_instance_id: run.runtime_vm_instance_id,
            attestation_status,
            lifecycle_state: "remediating",
            remediation_state: Some("remediation:automation-running"),
            remediation_attempts,
            freshness_deadline,
            provenance_ref: None,
            provenance: None,
            expected_version,
        },
    )
    .await?;
    tx.commit().await?;

    let pool_clone = pool.clone();
    let registry_clone = registry.clone();
    tokio::spawn(async move {
        execute_run(pool_clone, run, playbook, registry_clone).await;
    });
    Ok(Some(()))
}

async fn resolve_playbook(
    pool: &PgPool,
    run: &RuntimeVmRemediationRun,
) -> Result<Option<RuntimeVmRemediationPlaybook>, RemediationError> {
    if let Some(playbook_id) = run.playbook_id {
        Ok(get_playbook_by_id(pool, playbook_id).await?)
    } else {
        Ok(get_playbook_by_key(pool, &run.playbook).await?)
    }
}

async fn execute_run(
    pool: PgPool,
    run: RuntimeVmRemediationRun,
    playbook: Option<RuntimeVmRemediationPlaybook>,
    registry: Arc<RemediationExecutorRegistry>,
) {
    let executor_kind = playbook
        .as_ref()
        .map(|record| record.executor_type.as_str())
        .unwrap_or("shell");

    let Some(executor) = registry.get(executor_kind) else {
        error!(
            run_id = run.id,
            executor = executor_kind,
            "no remediation executor registered for playbook type"
        );
        let failure_reason = RemediationFailureReason::ExecutorUnavailable;
        if let Err(err) = finalize_failure(
            &pool,
            run,
            failure_reason,
            "executor not registered".to_string(),
            None,
        )
        .await
        {
            error!(?err, "failed to persist executor unavailable failure");
        }
        return;
    };

    let metadata = merge_metadata(&run, playbook.as_ref());
    let context = RemediationExecutionRequest {
        run: run.clone(),
        playbook,
        metadata,
    };

    match executor.execute(context).await {
        Ok(handle) => {
            let (mut log_rx, completion, _cancel) = handle.into_parts();
            let mut collected_logs = Vec::new();
            while let Some(event) = log_rx.recv().await {
                collected_logs.push(event.clone());
            }

            match completion.await {
                Ok(Ok(status)) if status.code == 0 => {
                    if let Err(err) = finalize_success(&pool, run, collected_logs, status).await {
                        error!(?err, "failed to persist remediation success");
                    }
                }
                Ok(Ok(status)) => {
                    let reason = status
                        .failure_reason
                        .unwrap_or(RemediationFailureReason::ExecutionFailure);
                    if let Err(err) = finalize_failure(
                        &pool,
                        run,
                        reason,
                        status.message.unwrap_or_else(|| {
                            "remediation executor returned non-zero exit".into()
                        }),
                        Some(collected_logs),
                    )
                    .await
                    {
                        error!(?err, "failed to persist remediation failure");
                    }
                }
                Ok(Err(err)) => {
                    let failure_reason = err.failure_reason();
                    if let Err(inner) = finalize_failure(
                        &pool,
                        run,
                        failure_reason,
                        err.to_string(),
                        Some(collected_logs),
                    )
                    .await
                    {
                        error!(?inner, "failed to persist remediation error outcome");
                    }
                }
                Err(join_err) => {
                    if let Err(inner) = finalize_failure(
                        &pool,
                        run,
                        RemediationFailureReason::TransientInfrastructure,
                        join_err.to_string(),
                        Some(collected_logs),
                    )
                    .await
                    {
                        error!(?inner, "failed to persist remediation join failure");
                    }
                }
            }
        }
        Err(err) => {
            let failure_reason = err.failure_reason();
            if let Err(inner) =
                finalize_failure(&pool, run, failure_reason, err.to_string(), None).await
            {
                error!(?inner, "failed to persist remediation spawn error");
            }
        }
    }
}

async fn handle_quarantine_event(
    pool: &PgPool,
    registry: &Arc<RemediationExecutorRegistry>,
    event: &TrustRegistryEvent,
) -> Result<(), RemediationError> {
    let mut tx = pool.begin().await?;
    let current_state = get_registry_state(&mut *tx, event.vm_instance_id).await?;

    if let Some(active_run) = get_active_run_for_instance(&mut *tx, event.vm_instance_id).await? {
        debug!(
            vm_instance_id = event.vm_instance_id,
            run_id = active_run.id,
            "active remediation run already staged; skipping duplicate enqueue",
        );
        tx.commit().await?;
        return Ok(());
    }

    let playbook = resolve_default_playbook(pool, registry).await?;
    let request = EnsureRemediationRunRequest {
        runtime_vm_instance_id: event.vm_instance_id,
        playbook_key: DEFAULT_PLAYBOOK,
        playbook_id: playbook.as_ref().map(|record| record.id),
        metadata: event.provenance.as_ref(),
        automation_payload: event.provenance.as_ref(),
        approval_required: playbook
            .as_ref()
            .map(|record| record.approval_required)
            .unwrap_or(false),
        assigned_owner_id: playbook.as_ref().map(|record| record.owner_id),
        sla_duration_seconds: playbook
            .as_ref()
            .and_then(|record| record.sla_duration_seconds),
    };

    if ensure_remediation_run(&mut *tx, request).await?.is_none() {
        tx.commit().await?;
        return Ok(());
    }

    let attempts = current_state
        .as_ref()
        .map(|state| state.remediation_attempts + 1)
        .unwrap_or(event.remediation_attempts + 1);

    let remediation_state = playbook
        .as_ref()
        .map(|record| {
            if record.approval_required {
                "remediation:pending-approval"
            } else {
                "remediation:awaiting-executor"
            }
        })
        .unwrap_or("remediation:awaiting-executor");

    let expected_version = current_state.as_ref().map(|state| state.version);
    upsert_registry_state(
        &mut *tx,
        UpsertRuntimeVmTrustRegistryState {
            runtime_vm_instance_id: event.vm_instance_id,
            attestation_status: event.attestation_status.as_str(),
            lifecycle_state: "remediating",
            remediation_state: Some(remediation_state),
            remediation_attempts: attempts,
            freshness_deadline: event.freshness_deadline,
            provenance_ref: event.provenance_ref.as_deref(),
            provenance: event.provenance.as_ref(),
            expected_version,
        },
    )
    .await?;

    tx.commit().await?;
    Ok(())
}

async fn resolve_default_playbook(
    pool: &PgPool,
    registry: &Arc<RemediationExecutorRegistry>,
) -> Result<Option<RuntimeVmRemediationPlaybook>, RemediationError> {
    let playbook = get_playbook_by_key(pool, DEFAULT_PLAYBOOK).await?;
    if let Some(ref record) = playbook {
        if registry.get(&record.executor_type).is_none() {
            warn!(
                playbook_key = %record.playbook_key,
                executor = %record.executor_type,
                "playbook references executor without registry mapping"
            );
        }
    }
    Ok(playbook)
}

fn merge_metadata(
    run: &RuntimeVmRemediationRun,
    playbook: Option<&RuntimeVmRemediationPlaybook>,
) -> Value {
    let mut root = json!({
        "run": {
            "id": run.id,
            "playbook_key": run.playbook,
            "assigned_owner_id": run.assigned_owner_id,
            "sla_deadline": run.sla_deadline.map(|deadline| deadline.to_rfc3339()),
        }
    });

    if let Some(existing) = root.get_mut("run") {
        if let Some(metadata) = existing.as_object_mut() {
            metadata.insert("metadata".into(), run.metadata.clone());
        }
    }

    if let Some(playbook) = playbook {
        if let Some(root_obj) = root.as_object_mut() {
            root_obj.insert(
                "playbook".into(),
                json!({
                    "id": playbook.id,
                    "key": playbook.playbook_key,
                    "executor_type": playbook.executor_type,
                    "sla_seconds": playbook.sla_duration_seconds,
                    "approval_required": playbook.approval_required,
                    "owner_id": playbook.owner_id,
                    "metadata": playbook.metadata,
                }),
            );
        }
    }

    root
}

async fn finalize_success(
    pool: &PgPool,
    run: RuntimeVmRemediationRun,
    logs: Vec<RemediationLogEvent>,
    status: RemediationExitStatus,
) -> Result<(), RemediationError> {
    let payload = json!({
        "exit_code": status.code,
        "message": status.message,
    });

    let metadata = json!({
        "logs": logs.clone(),
    });

    let mut tx = pool.begin().await?;
    if let Some(record) =
        mark_run_completed(&mut *tx, run.id, Some(&metadata), Some(&payload)).await?
    {
        let log_metadata = json!({
            "lines": logs,
            "summary": "remediation completed",
        });
        let _ = insert_artifact(
            &mut *tx,
            record.id,
            "execution-log",
            None,
            &log_metadata,
            None,
        )
        .await?;

        update_registry_after_completion(&mut tx, &run, "remediation:automation-complete").await?;
    }
    tx.commit().await?;
    info!(run_id = run.id, "remediation automation completed");
    Ok(())
}

async fn finalize_failure(
    pool: &PgPool,
    run: RuntimeVmRemediationRun,
    failure_reason: RemediationFailureReason,
    message: String,
    logs: Option<Vec<RemediationLogEvent>>,
) -> Result<(), RemediationError> {
    let metadata = logs.as_ref().map(|entries| json!({ "logs": entries }));
    let mut tx = pool.begin().await?;
    if let Some(record) = mark_run_failed(
        &mut *tx,
        run.id,
        failure_reason.as_str(),
        &message,
        metadata.as_ref(),
    )
    .await?
    {
        if let Some(entries) = logs {
            let artifact_metadata = json!({
                "lines": entries,
                "summary": message,
            });
            let _ = insert_artifact(
                &mut *tx,
                record.id,
                "execution-log",
                None,
                &artifact_metadata,
                None,
            )
            .await?;
        }

        update_registry_after_completion(
            &mut tx,
            &run,
            match failure_reason {
                RemediationFailureReason::Cancelled => "remediation:automation-cancelled",
                RemediationFailureReason::ExecutorUnavailable => "remediation:executor-unavailable",
                RemediationFailureReason::ExecutionFailure => "remediation:automation-failed",
                RemediationFailureReason::TransientInfrastructure => {
                    "remediation:transient-failure"
                }
            },
        )
        .await?;
    }
    tx.commit().await?;
    warn!(run_id = run.id, reason = %failure_reason.as_str(), message = %message, "remediation automation failed");
    Ok(())
}

async fn update_registry_after_completion(
    tx: &mut Transaction<'_, Postgres>,
    run: &RuntimeVmRemediationRun,
    remediation_state: &str,
) -> Result<(), RemediationError> {
    if let Some(state) = get_registry_state(&mut **tx, run.runtime_vm_instance_id).await? {
        let _ = upsert_registry_state(
            &mut **tx,
            UpsertRuntimeVmTrustRegistryState {
                runtime_vm_instance_id: run.runtime_vm_instance_id,
                attestation_status: state.attestation_status.as_str(),
                lifecycle_state: "remediating",
                remediation_state: Some(remediation_state),
                remediation_attempts: state.remediation_attempts,
                freshness_deadline: state.freshness_deadline,
                provenance_ref: state.provenance_ref.as_deref(),
                provenance: state.provenance.as_ref(),
                expected_version: Some(state.version),
            },
        )
        .await;
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct RemediationExecutionRequest {
    pub run: RuntimeVmRemediationRun,
    pub playbook: Option<RuntimeVmRemediationPlaybook>,
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize)]
pub enum RemediationLogStream {
    Stdout,
    Stderr,
    System,
}

#[derive(Debug, Clone, Serialize)]
pub struct RemediationLogEvent {
    pub timestamp: DateTime<Utc>,
    pub stream: RemediationLogStream,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RemediationExitStatus {
    pub code: i32,
    pub message: Option<String>,
    pub failure_reason: Option<RemediationFailureReason>,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub enum RemediationFailureReason {
    ExecutionFailure,
    TransientInfrastructure,
    Cancelled,
    ExecutorUnavailable,
}

impl RemediationFailureReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            RemediationFailureReason::ExecutionFailure => "execution-failure",
            RemediationFailureReason::TransientInfrastructure => "transient-infrastructure",
            RemediationFailureReason::Cancelled => "cancelled",
            RemediationFailureReason::ExecutorUnavailable => "executor-unavailable",
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum RemediationError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("executor unavailable")]
    ExecutorUnavailable,
    #[error("executor runtime error: {0}")]
    ExecutorRuntime(String, RemediationFailureReason),
    #[error("task join error: {0}")]
    Join(#[from] tokio::task::JoinError),
}

impl RemediationError {
    pub fn failure_reason(&self) -> RemediationFailureReason {
        match self {
            RemediationError::ExecutorUnavailable => RemediationFailureReason::ExecutorUnavailable,
            RemediationError::ExecutorRuntime(_, reason) => *reason,
            RemediationError::Database(_) => RemediationFailureReason::ExecutionFailure,
            RemediationError::Join(_) => RemediationFailureReason::TransientInfrastructure,
        }
    }
}

pub struct RemediationExecutionHandle {
    log_rx: mpsc::Receiver<RemediationLogEvent>,
    completion: JoinHandle<Result<RemediationExitStatus, RemediationError>>,
    cancel: Option<oneshot::Sender<()>>,
}

impl RemediationExecutionHandle {
    pub fn into_parts(
        self,
    ) -> (
        mpsc::Receiver<RemediationLogEvent>,
        JoinHandle<Result<RemediationExitStatus, RemediationError>>,
        Option<oneshot::Sender<()>>,
    ) {
        (self.log_rx, self.completion, self.cancel)
    }
}

#[async_trait]
pub trait RemediationExecutor: Send + Sync {
    fn kind(&self) -> RemediationExecutorKind;
    async fn execute(
        &self,
        context: RemediationExecutionRequest,
    ) -> Result<RemediationExecutionHandle, RemediationError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RemediationExecutorKind {
    Shell,
    Ansible,
    CloudApi,
}

impl RemediationExecutorKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            RemediationExecutorKind::Shell => "shell",
            RemediationExecutorKind::Ansible => "ansible",
            RemediationExecutorKind::CloudApi => "cloud_api",
        }
    }
}

impl FromStr for RemediationExecutorKind {
    type Err = RemediationError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        match input {
            "shell" => Ok(RemediationExecutorKind::Shell),
            "ansible" => Ok(RemediationExecutorKind::Ansible),
            "cloud_api" => Ok(RemediationExecutorKind::CloudApi),
            _ => Err(RemediationError::ExecutorUnavailable),
        }
    }
}

struct RemediationExecutorRegistry {
    executors: HashMap<RemediationExecutorKind, Arc<dyn RemediationExecutor>>,
}

impl RemediationExecutorRegistry {
    fn bootstrap() -> Self {
        let mut executors: HashMap<RemediationExecutorKind, Arc<dyn RemediationExecutor>> =
            HashMap::new();
        executors.insert(
            RemediationExecutorKind::Shell,
            Arc::new(ShellRemediationExecutor),
        );
        executors.insert(
            RemediationExecutorKind::Ansible,
            Arc::new(AnsibleRemediationExecutor),
        );
        executors.insert(
            RemediationExecutorKind::CloudApi,
            Arc::new(CloudApiRemediationExecutor),
        );
        Self { executors }
    }

    fn get<T: AsRef<str>>(&self, kind: T) -> Option<Arc<dyn RemediationExecutor>> {
        let key = RemediationExecutorKind::from_str(kind.as_ref()).ok()?;
        self.executors.get(&key).cloned()
    }
}

struct ShellRemediationExecutor;
struct AnsibleRemediationExecutor;
struct CloudApiRemediationExecutor;

#[async_trait]
impl RemediationExecutor for ShellRemediationExecutor {
    fn kind(&self) -> RemediationExecutorKind {
        RemediationExecutorKind::Shell
    }

    async fn execute(
        &self,
        context: RemediationExecutionRequest,
    ) -> Result<RemediationExecutionHandle, RemediationError> {
        execute_simulated("shell", context, Duration::from_secs(5)).await
    }
}

#[async_trait]
impl RemediationExecutor for AnsibleRemediationExecutor {
    fn kind(&self) -> RemediationExecutorKind {
        RemediationExecutorKind::Ansible
    }

    async fn execute(
        &self,
        context: RemediationExecutionRequest,
    ) -> Result<RemediationExecutionHandle, RemediationError> {
        execute_simulated("ansible", context, Duration::from_secs(7)).await
    }
}

#[async_trait]
impl RemediationExecutor for CloudApiRemediationExecutor {
    fn kind(&self) -> RemediationExecutorKind {
        RemediationExecutorKind::CloudApi
    }

    async fn execute(
        &self,
        context: RemediationExecutionRequest,
    ) -> Result<RemediationExecutionHandle, RemediationError> {
        execute_simulated("cloud_api", context, Duration::from_secs(3)).await
    }
}

async fn execute_simulated(
    executor: &str,
    context: RemediationExecutionRequest,
    duration: Duration,
) -> Result<RemediationExecutionHandle, RemediationError> {
    let (log_tx, log_rx) = mpsc::channel(REMEDIATION_STREAM_BUFFER);
    let (cancel_tx, cancel_rx) = oneshot::channel();
    let run_id = context.run.id;
    let metadata = context.metadata;
    let executor_label = executor.to_string();
    let join: JoinHandle<Result<RemediationExitStatus, RemediationError>> = tokio::spawn({
        let mut cancel_rx = cancel_rx;
        async move {
            let executor = executor_label;
            let mut cancelled = false;
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            let mut elapsed = 0;
            loop {
                tokio::select! {
                    _ = tick.tick() => {
                        elapsed += 1;
                        if elapsed as u64 <= duration.as_secs() {
                            let _ = log_tx.send(RemediationLogEvent {
                                timestamp: Utc::now(),
                                stream: RemediationLogStream::Stdout,
                                message: format!("[{executor}] remediation tick {elapsed} for run {run_id}"),
                            }).await;
                        } else {
                            break;
                        }
                    }
                    _ = &mut cancel_rx => {
                        cancelled = true;
                        break;
                    }
                }
            }

            if cancelled {
                let _ = log_tx
                    .send(RemediationLogEvent {
                        timestamp: Utc::now(),
                        stream: RemediationLogStream::System,
                        message: format!("[{executor}] remediation cancelled for run {run_id}"),
                    })
                    .await;
                drop(log_tx);
                Ok(RemediationExitStatus {
                    code: 1,
                    message: Some("remediation cancelled".into()),
                    failure_reason: Some(RemediationFailureReason::Cancelled),
                })
            } else {
                let _ = log_tx
                    .send(RemediationLogEvent {
                        timestamp: Utc::now(),
                        stream: RemediationLogStream::Stdout,
                        message: format!("[{executor}] remediation completed for run {run_id}"),
                    })
                    .await;
                let _ = log_tx
                    .send(RemediationLogEvent {
                        timestamp: Utc::now(),
                        stream: RemediationLogStream::System,
                        message: format!("[{executor}] metadata: {}", metadata),
                    })
                    .await;
                drop(log_tx);
                Ok(RemediationExitStatus {
                    code: 0,
                    message: Some("remediation completed successfully".into()),
                    failure_reason: None,
                })
            }
        }
    });

    Ok(RemediationExecutionHandle {
        log_rx,
        completion: join,
        cancel: Some(cancel_tx),
    })
}
