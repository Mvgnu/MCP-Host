use anyhow::{bail, Context, Result};
use axum::{
    body::{Body, Bytes, HttpBody},
    http::{Method, Request, StatusCode},
    response::Response,
    routing::{get, post},
    Extension, Router,
};
use backend::db::runtime_vm_remediation_artifacts::insert_artifact;
use backend::db::runtime_vm_remediation_runs::{mark_run_completed, mark_run_failed};
use backend::db::runtime_vm_trust_registry::{upsert_state, UpsertRuntimeVmTrustRegistryState};
use backend::policy::trust::evaluate_placement_gate;
use chrono::{Duration as ChronoDuration, Utc};
use futures_util::future::join_all;
use hyper::body;
use jsonwebtoken::{encode, EncodingKey, Header};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    time::Duration as StdDuration,
};
use tokio::time::timeout;
use tower::ServiceExt;

#[derive(Clone)]
struct RemediationHarness {
    app: Router,
    pool: PgPool,
    token: String,
    operator_id: i32,
    server_id: i32,
    vm_instance_id: i64,
}

fn generate_operator_token(operator_id: i32) -> String {
    let exp = (Utc::now() + ChronoDuration::hours(1)).timestamp();
    let claims = json!({"sub": operator_id, "role": "operator", "exp": exp});
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(b"integration-secret"),
    )
    .unwrap()
}

async fn bootstrap_remediation_harness(pool: &PgPool) -> RemediationHarness {
    sqlx::migrate!("./migrations").run(pool).await.unwrap();

    std::env::set_var("JWT_SECRET", "integration-secret");

    let operator_id: i32 =
        sqlx::query_scalar("INSERT INTO users (email, password_hash) VALUES ($1, $2) RETURNING id")
            .bind("remediator@example.com")
            .bind("hashed")
            .fetch_one(pool)
            .await
            .unwrap();

    let server_id: i32 = sqlx::query_scalar(
        "INSERT INTO mcp_servers (owner_id, name, server_type, config, status, api_key) VALUES ($1, $2, $3, '{}'::jsonb, $4, $5) RETURNING id",
    )
    .bind(operator_id)
    .bind("edge-remediation")
    .bind("virtual-machine")
    .bind("active")
    .bind("test-key")
    .fetch_one(pool)
    .await
    .unwrap();

    let vm_instance_id: i64 = sqlx::query_scalar(
        "INSERT INTO runtime_vm_instances (server_id, instance_id) VALUES ($1, $2) RETURNING id",
    )
    .bind(server_id)
    .bind("vm-remediation-1")
    .fetch_one(pool)
    .await
    .unwrap();

    let token = generate_operator_token(operator_id);

    let app = Router::new()
        .route(
            "/api/trust/remediation/workspaces",
            get(backend::remediation_api::list_workspaces_handler)
                .post(backend::remediation_api::create_workspace_handler),
        )
        .route(
            "/api/trust/remediation/workspaces/:workspace_id",
            get(backend::remediation_api::get_workspace_handler),
        )
        .route(
            "/api/trust/remediation/workspaces/:workspace_id/revisions",
            post(backend::remediation_api::create_workspace_revision_handler),
        )
        .route(
            "/api/trust/remediation/workspaces/:workspace_id/revisions/:revision_id/schema",
            post(backend::remediation_api::apply_workspace_schema_validation_handler),
        )
        .route(
            "/api/trust/remediation/workspaces/:workspace_id/revisions/:revision_id/policy",
            post(backend::remediation_api::apply_workspace_policy_feedback_handler),
        )
        .route(
            "/api/trust/remediation/workspaces/:workspace_id/revisions/:revision_id/simulation",
            post(backend::remediation_api::apply_workspace_simulation_handler),
        )
        .route(
            "/api/trust/remediation/workspaces/:workspace_id/revisions/:revision_id/promotion",
            post(backend::remediation_api::apply_workspace_promotion_handler),
        )
        .route(
            "/api/trust/remediation/playbooks",
            get(backend::remediation_api::list_all_playbooks)
                .post(backend::remediation_api::create_playbook_handler),
        )
        .route(
            "/api/trust/remediation/playbooks/:playbook_id",
            get(backend::remediation_api::get_playbook_handler)
                .patch(backend::remediation_api::update_playbook_handler)
                .delete(backend::remediation_api::delete_playbook_handler),
        )
        .route(
            "/api/trust/remediation/runs",
            get(backend::remediation_api::list_runs_handler)
                .post(backend::remediation_api::enqueue_run_handler),
        )
        .route(
            "/api/trust/remediation/runs/:run_id",
            get(backend::remediation_api::get_run_handler),
        )
        .route(
            "/api/trust/remediation/runs/:run_id/approval",
            post(backend::remediation_api::update_approval_handler),
        )
        .route(
            "/api/trust/remediation/runs/:run_id/artifacts",
            get(backend::remediation_api::list_artifacts_handler),
        )
        .route(
            "/api/trust/remediation/stream",
            get(backend::remediation_api::stream_remediation_events),
        )
        .layer(Extension(pool.clone()));

    backend::remediation::spawn(pool.clone());

    RemediationHarness {
        app,
        pool: pool.clone(),
        token,
        operator_id,
        server_id,
        vm_instance_id,
    }
}

impl RemediationHarness {
    fn issue_token(&self, operator_id: i32) -> String {
        generate_operator_token(operator_id)
    }

    async fn create_operator(&self, email: &str) -> (i32, String) {
        let operator_id: i32 = sqlx::query_scalar(
            "INSERT INTO users (email, password_hash) VALUES ($1, $2) RETURNING id",
        )
        .bind(email)
        .bind("hashed")
        .fetch_one(&self.pool)
        .await
        .unwrap();

        let token = self.issue_token(operator_id);
        (operator_id, token)
    }

    async fn create_server_and_vm(
        &self,
        owner_id: i32,
        server_name: &str,
        instance_id: &str,
    ) -> (i32, i64) {
        let server_id: i32 = sqlx::query_scalar(
            "INSERT INTO mcp_servers (owner_id, name, server_type, config, status, api_key) VALUES ($1, $2, $3, '{}'::jsonb, $4, $5) RETURNING id",
        )
        .bind(owner_id)
        .bind(server_name)
        .bind("virtual-machine")
        .bind("active")
        .bind("test-key")
        .fetch_one(&self.pool)
        .await
        .unwrap();

        let vm_instance_id: i64 = sqlx::query_scalar(
            "INSERT INTO runtime_vm_instances (server_id, instance_id) VALUES ($1, $2) RETURNING id",
        )
        .bind(server_id)
        .bind(instance_id)
        .fetch_one(&self.pool)
        .await
        .unwrap();

        (server_id, vm_instance_id)
    }

    fn pool(&self) -> &PgPool {
        &self.pool
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScenarioKind {
    TenantIsolation,
    ConcurrentApprovals,
    ExecutorOutageResumption,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ScenarioDefinition {
    name: String,
    tag: String,
    kind: ScenarioKind,
    metadata: Value,
}

fn merge_metadata_fields(target: &mut Value, extras: &Value) {
    if let (Some(target_map), Some(extra_map)) = (target.as_object_mut(), extras.as_object()) {
        for (key, value) in extra_map {
            target_map.insert(key.clone(), value.clone());
        }
    }
}

#[derive(Debug, Deserialize)]
struct ScenarioManifestDocument {
    #[serde(default)]
    description: Option<String>,
    scenarios: Vec<ScenarioManifestEntry>,
}

#[derive(Debug, Deserialize)]
struct ScenarioManifestEntry {
    name: String,
    tag: String,
    kind: ScenarioManifestKind,
    #[serde(default)]
    tenants: BTreeSet<String>,
    #[serde(default)]
    metadata: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum ScenarioManifestKind {
    TenantIsolation,
    ConcurrentApprovals,
    ExecutorOutageResumption,
}

#[derive(Clone, Debug)]
struct ScenarioExecution {
    definition: ScenarioDefinition,
    tenant: String,
}

const SCENARIO_DIR_ENV: &str = "REM_FABRIC_SCENARIO_DIR";
const DEFAULT_SCENARIO_DIR: &str = "../scripts/remediation_harness/scenarios";

// key: verification -> remediation-fabric:manifest-loader
fn scenario_kind_from_manifest(kind: ScenarioManifestKind) -> ScenarioKind {
    match kind {
        ScenarioManifestKind::TenantIsolation => ScenarioKind::TenantIsolation,
        ScenarioManifestKind::ConcurrentApprovals => ScenarioKind::ConcurrentApprovals,
        ScenarioManifestKind::ExecutorOutageResumption => ScenarioKind::ExecutorOutageResumption,
    }
}

fn resolve_manifest_root() -> PathBuf {
    std::env::var(SCENARIO_DIR_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|_| Path::new(env!("CARGO_MANIFEST_DIR")).join(DEFAULT_SCENARIO_DIR))
}

fn load_manifest_directory(path: &Path) -> Result<Vec<ScenarioExecution>> {
    if !path.exists() {
        bail!("scenario manifest directory missing: {}", path.display());
    }

    let mut manifest_paths: Vec<PathBuf> = fs::read_dir(path)
        .with_context(|| format!("reading scenario manifest directory {}", path.display()))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|candidate| candidate.is_file())
        .collect();
    manifest_paths.sort();

    let mut executions = Vec::new();
    for manifest_path in manifest_paths {
        executions.extend(load_manifest_file(&manifest_path)?);
    }

    if executions.is_empty() {
        bail!(
            "no scenarios discovered in manifest directory {}",
            path.display()
        );
    }

    Ok(executions)
}

fn load_manifest_file(path: &Path) -> Result<Vec<ScenarioExecution>> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("reading scenario manifest {}", path.display()))?;

    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    let document: ScenarioManifestDocument = match extension.as_str() {
        "json" => serde_json::from_str(&raw)
            .with_context(|| format!("parsing JSON manifest {}", path.display()))?,
        "yaml" | "yml" => serde_yaml::from_str(&raw)
            .with_context(|| format!("parsing YAML manifest {}", path.display()))?,
        other => bail!(
            "unsupported manifest extension {} for {}",
            other,
            path.display()
        ),
    };

    let mut executions = Vec::new();
    for entry in document.scenarios {
        let definition = ScenarioDefinition {
            name: entry.name.clone(),
            tag: entry.tag.clone(),
            kind: scenario_kind_from_manifest(entry.kind),
            metadata: entry.metadata.clone(),
        };

        let tenants = if entry.tenants.is_empty() {
            vec!["default".to_string()]
        } else {
            entry.tenants.into_iter().collect()
        };

        for tenant in tenants {
            executions.push(ScenarioExecution {
                definition: definition.clone(),
                tenant,
            });
        }
    }

    Ok(executions)
}

async fn run_scenario(harness: &RemediationHarness, scenario: &ScenarioDefinition, tenant: &str) {
    eprintln!(
        "[remediation-chaos] executing scenario {} ({})",
        scenario.name, scenario.tag
    );
    match scenario.kind {
        ScenarioKind::TenantIsolation => {
            // key: validation -> remediation-matrix:tenant-isolation
            scenario_tenant_isolation(harness, scenario, tenant).await;
        }
        ScenarioKind::ConcurrentApprovals => {
            // key: validation -> remediation-matrix:concurrent-approvals
            scenario_concurrent_approvals(harness, scenario, tenant).await;
        }
        ScenarioKind::ExecutorOutageResumption => {
            // key: validation -> remediation-matrix:executor-outage
            scenario_executor_outage_resumption(harness, scenario, tenant).await;
        }
    }
}

async fn scenario_tenant_isolation(
    harness: &RemediationHarness,
    scenario: &ScenarioDefinition,
    tenant: &str,
) {
    let app = harness.app.clone();
    let primary_token = harness.token.clone();
    let primary_vm = harness.vm_instance_id;
    let primary_server = harness.server_id;

    let email = format!("{}+{}@example.com", scenario.name, tenant);
    let (secondary_operator, secondary_token) = harness.create_operator(&email).await;
    let (secondary_server, secondary_vm) = harness
        .create_server_and_vm(
            secondary_operator,
            &format!("{}-{}", tenant, scenario.name),
            &format!("{}-vm-b-{}", scenario.name, tenant),
        )
        .await;

    let playbook_key = format!("vm.restart.{}.{}", scenario.name, tenant);
    let scenario_tag = format!("{}::{}", scenario.tag, tenant);
    let playbook_payload = json!({
        "playbook_key": playbook_key,
        "display_name": format!("Restart VM - {} ({tenant})", scenario.name),
        "description": "Restart workload",
        "executor_type": "shell",
        "approval_required": true,
        "sla_duration_seconds": 900,
        "metadata": {"tier": "gold", "scenario": scenario_tag.clone()}
    });
    let playbook = create_playbook(&app, &primary_token, playbook_payload).await;
    let playbook_id = playbook["id"].as_i64().unwrap();

    let enqueue_payload_primary = json!({
        "runtime_vm_instance_id": primary_vm,
        "playbook": playbook_key,
        "assigned_owner_id": harness.operator_id,
        "metadata": {"scenario": scenario_tag.clone(), "tenant": format!("primary::{tenant}")},
        "automation_payload": null
    });
    let primary_run = enqueue_run(&app, &primary_token, enqueue_payload_primary).await;
    assert_eq!(
        primary_run["run"]["playbook_id"].as_i64(),
        Some(playbook_id)
    );

    let enqueue_payload_secondary = json!({
        "runtime_vm_instance_id": secondary_vm,
        "playbook": playbook_key,
        "assigned_owner_id": secondary_operator,
        "metadata": {"scenario": scenario_tag.clone(), "tenant": format!("secondary::{tenant}")},
        "automation_payload": null
    });
    let secondary_run = enqueue_run(&app, &secondary_token, enqueue_payload_secondary).await;
    assert_eq!(
        secondary_run["run"]["runtime_vm_instance_id"].as_i64(),
        Some(secondary_vm)
    );

    let scoped_runs = list_runs_for_instance(&app, &primary_token, primary_vm).await;
    assert_eq!(scoped_runs.len(), 1);
    assert_eq!(
        scoped_runs[0]["runtime_vm_instance_id"].as_i64(),
        Some(primary_vm)
    );

    let scoped_secondary = list_runs_for_instance(&app, &secondary_token, secondary_vm).await;
    assert_eq!(scoped_secondary.len(), 1);
    assert_eq!(
        scoped_secondary[0]["runtime_vm_instance_id"].as_i64(),
        Some(secondary_vm)
    );

    let primary_state = format!("remediation:pending:{}:{}", scenario.name, tenant);
    upsert_state(
        harness.pool(),
        UpsertRuntimeVmTrustRegistryState {
            runtime_vm_instance_id: primary_vm,
            attestation_status: "untrusted",
            lifecycle_state: "remediating",
            remediation_state: Some(primary_state.as_str()),
            remediation_attempts: 1,
            freshness_deadline: None,
            provenance_ref: None,
            provenance: None,
            expected_version: None,
        },
    )
    .await
    .unwrap();

    let secondary_state = format!("remediation:pending:{}:{}", scenario.tag, tenant);
    upsert_state(
        harness.pool(),
        UpsertRuntimeVmTrustRegistryState {
            runtime_vm_instance_id: secondary_vm,
            attestation_status: "untrusted",
            lifecycle_state: "remediating",
            remediation_state: Some(secondary_state.as_str()),
            remediation_attempts: 1,
            freshness_deadline: None,
            provenance_ref: None,
            provenance: None,
            expected_version: None,
        },
    )
    .await
    .unwrap();

    let primary_gate = evaluate_placement_gate(harness.pool(), primary_server)
        .await
        .unwrap()
        .unwrap();
    assert!(primary_gate.blocked);
    assert_eq!(
        primary_gate.remediation_state.as_deref(),
        Some(primary_state.as_str())
    );

    let secondary_gate = evaluate_placement_gate(harness.pool(), secondary_server)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("no gate for server {}", secondary_server));
    assert!(secondary_gate.blocked);
    assert_eq!(
        secondary_gate.remediation_state.as_deref(),
        Some(secondary_state.as_str())
    );
}

async fn scenario_concurrent_approvals(
    harness: &RemediationHarness,
    scenario: &ScenarioDefinition,
    tenant: &str,
) {
    let app = harness.app.clone();
    let playbook_key = format!("vm.approval.{}.{}", scenario.name, tenant);
    let scenario_tag = format!("{}::{}", scenario.tag, tenant);
    let playbook_payload = json!({
        "playbook_key": playbook_key,
        "display_name": "Approval Stress",
        "description": "Concurrency approval scenario",
        "executor_type": "shell",
        "approval_required": true,
        "sla_duration_seconds": 600,
        "metadata": {"scenario": scenario_tag.clone()}
    });

    create_playbook(&app, &harness.token, playbook_payload).await;
    let enqueue_payload = json!({
        "runtime_vm_instance_id": harness.vm_instance_id,
        "playbook": playbook_key,
        "metadata": {"scenario": scenario_tag},
        "automation_payload": null
    });
    let run = enqueue_run(&app, &harness.token, enqueue_payload).await;
    let run_id = run["run"]["id"].as_i64().unwrap();
    let run_version = run["run"]["version"].as_i64().unwrap();

    let approval_payload = json!({
        "new_state": "approved",
        "approval_notes": "concurrent-approval",
        "expected_version": run_version
    });
    let approval_body = approval_payload.to_string();

    let app_first = app.clone();
    let app_second = app.clone();
    let token_first = harness.token.clone();
    let token_second = harness.token.clone();
    let uri = format!("/api/trust/remediation/runs/{run_id}/approval");

    let (resp_a, resp_b) = tokio::join!(
        post_json(&app_first, &token_first, &uri, approval_body.clone()),
        post_json(&app_second, &token_second, &uri, approval_body),
    );

    let statuses = [resp_a.status(), resp_b.status()];
    assert!(statuses.contains(&StatusCode::OK));
    assert!(statuses.contains(&StatusCode::CONFLICT));

    let success_response = if resp_a.status() == StatusCode::OK {
        resp_a
    } else {
        resp_b
    };
    let body_bytes = body::to_bytes(success_response.into_body()).await.unwrap();
    let updated_run: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(updated_run["approval_state"], "approved");
}

async fn scenario_executor_outage_resumption(
    harness: &RemediationHarness,
    scenario: &ScenarioDefinition,
    tenant: &str,
) {
    let app = harness.app.clone();
    let playbook_key = format!("vm.executor.{}.{}", scenario.name, tenant);
    let scenario_tag = format!("{}::{}", scenario.tag, tenant);
    let mut playbook_metadata = json!({"scenario": scenario_tag.clone()});
    merge_metadata_fields(&mut playbook_metadata, &scenario.metadata);
    let playbook_payload = json!({
        "playbook_key": playbook_key,
        "display_name": "Executor Outage",
        "description": "Simulated executor outage",
        "executor_type": "shell",
        "approval_required": false,
        "sla_duration_seconds": 300,
        "metadata": playbook_metadata
    });

    create_playbook(&app, &harness.token, playbook_payload).await;
    let mut initial_metadata = json!({
        "scenario": scenario_tag.clone(),
        "phase": "initial"
    });
    merge_metadata_fields(&mut initial_metadata, &scenario.metadata);
    let enqueue_payload = json!({
        "runtime_vm_instance_id": harness.vm_instance_id,
        "playbook": playbook_key,
        "metadata": initial_metadata,
        "automation_payload": null
    });
    let run = enqueue_run(&app, &harness.token, enqueue_payload).await;
    let run_id = run["run"]["id"].as_i64().unwrap();

    sqlx::query("UPDATE runtime_vm_remediation_runs SET status = 'running' WHERE id = $1")
        .bind(run_id)
        .execute(harness.pool())
        .await
        .unwrap();

    let failure_metadata = json!({
        "scenario": scenario_tag.clone(),
        "failure": "executor-unavailable",
        "tenant": tenant
    });
    mark_run_failed(
        harness.pool(),
        run_id,
        "executor_unavailable",
        "executor not registered",
        Some(&failure_metadata),
    )
    .await
    .unwrap();

    let outage_state = format!("remediation:executor-outage:{}:{}", scenario.name, tenant);

    let registry_state = upsert_state(
        harness.pool(),
        UpsertRuntimeVmTrustRegistryState {
            runtime_vm_instance_id: harness.vm_instance_id,
            attestation_status: "untrusted",
            lifecycle_state: "remediating",
            remediation_state: Some(outage_state.as_str()),
            remediation_attempts: 1,
            freshness_deadline: None,
            provenance_ref: None,
            provenance: None,
            expected_version: None,
        },
    )
    .await
    .unwrap();

    let gate = evaluate_placement_gate(harness.pool(), harness.server_id)
        .await
        .unwrap()
        .unwrap();
    assert!(gate.blocked);
    assert!(gate
        .notes
        .iter()
        .any(|note| note.contains("executor-outage")));

    let mut retry_metadata = json!({
        "scenario": scenario_tag.clone(),
        "phase": "retry"
    });
    merge_metadata_fields(&mut retry_metadata, &scenario.metadata);
    let retry_payload = json!({
        "runtime_vm_instance_id": harness.vm_instance_id,
        "playbook": playbook_key,
        "metadata": retry_metadata,
        "automation_payload": null
    });
    let retry_run = enqueue_run(&app, &harness.token, retry_payload).await;
    assert_eq!(retry_run["created"], true);
    let retry_run_id = retry_run["run"]["id"].as_i64().unwrap();

    sqlx::query("UPDATE runtime_vm_remediation_runs SET status = 'running' WHERE id = $1")
        .bind(retry_run_id)
        .execute(harness.pool())
        .await
        .unwrap();

    let completion_metadata = json!({
        "scenario": scenario_tag.clone(),
        "phase": "recovery",
        "notes": "executor restored",
        "tenant": tenant
    });
    mark_run_completed(
        harness.pool(),
        retry_run_id,
        Some(&completion_metadata),
        None,
    )
    .await
    .unwrap();

    let restored_state = format!(
        "remediation:automation-complete:{}:{}",
        scenario.name, tenant
    );

    upsert_state(
        harness.pool(),
        UpsertRuntimeVmTrustRegistryState {
            runtime_vm_instance_id: harness.vm_instance_id,
            attestation_status: "trusted",
            lifecycle_state: "restored",
            remediation_state: Some(restored_state.as_str()),
            remediation_attempts: registry_state.remediation_attempts + 1,
            freshness_deadline: None,
            provenance_ref: None,
            provenance: None,
            expected_version: Some(registry_state.version),
        },
    )
    .await
    .unwrap();

    let failed_status: (String,) =
        sqlx::query_as("SELECT status FROM runtime_vm_remediation_runs WHERE id = $1")
            .bind(run_id)
            .fetch_one(harness.pool())
            .await
            .unwrap();
    assert_eq!(failed_status.0, "failed");

    let completed_status: (String,) =
        sqlx::query_as("SELECT status FROM runtime_vm_remediation_runs WHERE id = $1")
            .bind(retry_run_id)
            .fetch_one(harness.pool())
            .await
            .unwrap();
    assert_eq!(completed_status.0, "completed");

    let queued_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM runtime_vm_remediation_runs WHERE runtime_vm_instance_id = $1 AND status = 'queued'",
    )
    .bind(harness.vm_instance_id)
    .fetch_one(harness.pool())
    .await
    .unwrap();
    assert_eq!(queued_count.0, 0);

    let restored_gate = evaluate_placement_gate(harness.pool(), harness.server_id)
        .await
        .unwrap()
        .unwrap();
    assert!(!restored_gate.blocked);
}

async fn post_workspace_request(
    app: &Router,
    token: &str,
    uri: String,
    payload: Value,
) -> Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(uri)
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn create_workspace(app: &Router, token: &str, payload: Value) -> Value {
    let response = post_workspace_request(
        app,
        token,
        "/api/trust/remediation/workspaces".to_string(),
        payload,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = body::to_bytes(response.into_body()).await.unwrap();
    serde_json::from_slice(&body_bytes).unwrap()
}

async fn create_workspace_revision(
    app: &Router,
    token: &str,
    workspace_id: i64,
    payload: Value,
) -> Value {
    let response = post_workspace_request(
        app,
        token,
        format!("/api/trust/remediation/workspaces/{workspace_id}/revisions"),
        payload,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = body::to_bytes(response.into_body()).await.unwrap();
    serde_json::from_slice(&body_bytes).unwrap()
}

async fn apply_workspace_schema(
    app: &Router,
    token: &str,
    workspace_id: i64,
    revision_id: i64,
    payload: Value,
) -> Value {
    let response = post_workspace_request(
        app,
        token,
        format!("/api/trust/remediation/workspaces/{workspace_id}/revisions/{revision_id}/schema"),
        payload,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = body::to_bytes(response.into_body()).await.unwrap();
    serde_json::from_slice(&body_bytes).unwrap()
}

async fn apply_workspace_policy(
    app: &Router,
    token: &str,
    workspace_id: i64,
    revision_id: i64,
    payload: Value,
) -> Value {
    let response = post_workspace_request(
        app,
        token,
        format!("/api/trust/remediation/workspaces/{workspace_id}/revisions/{revision_id}/policy"),
        payload,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = body::to_bytes(response.into_body()).await.unwrap();
    serde_json::from_slice(&body_bytes).unwrap()
}

async fn apply_workspace_simulation(
    app: &Router,
    token: &str,
    workspace_id: i64,
    revision_id: i64,
    payload: Value,
) -> Value {
    let response = post_workspace_request(
        app,
        token,
        format!(
            "/api/trust/remediation/workspaces/{workspace_id}/revisions/{revision_id}/simulation"
        ),
        payload,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = body::to_bytes(response.into_body()).await.unwrap();
    serde_json::from_slice(&body_bytes).unwrap()
}

async fn apply_workspace_promotion(
    app: &Router,
    token: &str,
    workspace_id: i64,
    revision_id: i64,
    payload: Value,
) -> Value {
    let response = post_workspace_request(
        app,
        token,
        format!(
            "/api/trust/remediation/workspaces/{workspace_id}/revisions/{revision_id}/promotion"
        ),
        payload,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = body::to_bytes(response.into_body()).await.unwrap();
    serde_json::from_slice(&body_bytes).unwrap()
}

async fn fetch_workspace_details(app: &Router, token: &str, workspace_id: i64) -> Value {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/api/trust/remediation/workspaces/{workspace_id}"))
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = body::to_bytes(response.into_body()).await.unwrap();
    serde_json::from_slice(&body_bytes).unwrap()
}

async fn list_workspaces(app: &Router, token: &str) -> Vec<Value> {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/trust/remediation/workspaces")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = body::to_bytes(response.into_body()).await.unwrap();
    serde_json::from_slice(&body_bytes).unwrap()
}

async fn list_workspace_runs(
    app: &Router,
    token: &str,
    workspace_id: i64,
    revision_id: i64,
) -> Vec<Value> {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!(
                    "/api/trust/remediation/runs?workspace_id={workspace_id}&workspace_revision_id={revision_id}"
                ))
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = body::to_bytes(response.into_body()).await.unwrap();
    serde_json::from_slice(&body_bytes).unwrap()
}

fn select_revision<'a>(envelope: &'a Value, revision_id: i64) -> &'a Value {
    envelope["revisions"]
        .as_array()
        .expect("revisions array")
        .iter()
        .find(|entry| entry["revision"]["id"].as_i64() == Some(revision_id))
        .expect("revision not found")
}

fn find_snapshot<'a>(revision: &'a Value, snapshot_type: &str) -> Option<&'a Value> {
    revision["validation_snapshots"]
        .as_array()
        .unwrap()
        .iter()
        .find(|snapshot| snapshot["snapshot_type"].as_str() == Some(snapshot_type))
}

// key: validation -> remediation-workspace-draft
// key: validation -> remediation-workspace-promotion
#[sqlx::test]
#[ignore = "requires DATABASE_URL with Postgres server"]
async fn remediation_workspace_lifecycle_end_to_end(pool: PgPool) {
    let harness = bootstrap_remediation_harness(&pool).await;
    let app = harness.app.clone();
    let token = harness.token.clone();
    let vm_instance_id = harness.vm_instance_id;

    let restart_playbook = json!({
        "playbook_key": "vm.restart",
        "display_name": "VM Restart",
        "description": "Restart target VM instances",
        "executor_type": "shell",
        "approval_required": false,
        "metadata": {"origin": "workspace-lifecycle"},
    });
    create_playbook(&app, &token, restart_playbook).await;

    let workspace_payload = json!({
        "workspace_key": "workspace.alpha",
        "display_name": "Workspace Alpha",
        "description": "Draft remediation workspace",
        "plan": {
            "playbooks": ["vm.restart"],
            "targets": [
                {
                    "runtime_vm_instance_id": vm_instance_id,
                    "playbook": "vm.restart",
                    "automation_payload": {
                        "trigger": "workspace-draft",
                        "reason": "initial baseline",
                    },
                }
            ]
        },
        "metadata": {"source": "integration"},
        "lineage_tags": ["validation:remediation-workspace-draft"],
        "lineage_labels": ["channel:alpha"],
    });

    let workspace = create_workspace(&app, &token, workspace_payload).await;
    let workspace_id = workspace["workspace"]["id"].as_i64().unwrap();
    let mut workspace_version = workspace["workspace"]["version"].as_i64().unwrap();
    let active_revision_id = workspace["workspace"]["active_revision_id"]
        .as_i64()
        .unwrap();
    let initial_revision = select_revision(&workspace, active_revision_id);
    assert_eq!(
        initial_revision["revision"]["revision_number"].as_i64(),
        Some(1)
    );
    assert_eq!(
        initial_revision["gate_summary"]["schema_status"].as_str(),
        Some("pending")
    );

    let initial_revision_version = initial_revision["revision"]["version"].as_i64().unwrap();

    let revision_payload = json!({
        "plan": {
            "playbooks": ["vm.restart", "vm.redeploy"],
            "targets": [
                {
                    "runtime_vm_instance_id": vm_instance_id,
                    "playbook": "vm.restart",
                    "automation_payload": {
                        "trigger": "workspace-revision",
                        "reason": "v2 rollout",
                    },
                }
            ]
        },
        "metadata": {"change": "v2"},
        "lineage_labels": ["channel:alpha", "experiment:v2"],
        "expected_workspace_version": workspace_version,
        "previous_revision_id": initial_revision["revision"]["id"].as_i64(),
    });

    let updated = create_workspace_revision(&app, &token, workspace_id, revision_payload).await;
    workspace_version = updated["workspace"]["version"].as_i64().unwrap();
    let latest_revision = updated["revisions"].as_array().unwrap()[0].clone();
    let latest_revision_id = latest_revision["revision"]["id"].as_i64().unwrap();
    let mut revision_version = latest_revision["revision"]["version"].as_i64().unwrap();

    let stale_revision_response = post_workspace_request(
        &app,
        &token,
        format!("/api/trust/remediation/workspaces/{workspace_id}/revisions"),
        json!({
            "plan": {"playbooks": ["vm.restart"]},
            "metadata": {"change": "stale"},
            "lineage_labels": [],
            "expected_workspace_version": workspace_version - 1,
            "previous_revision_id": latest_revision["revision"]["id"].as_i64(),
        }),
    )
    .await;
    assert_eq!(stale_revision_response.status(), StatusCode::CONFLICT);

    let schema_payload = json!({
        "result_status": "passed",
        "errors": [],
        "gate_context": {"validator": "schema-bot"},
        "metadata": {"token": "schema-v1"},
        "expected_revision_version": revision_version,
    });

    let after_schema = apply_workspace_schema(
        &app,
        &token,
        workspace_id,
        latest_revision_id,
        schema_payload,
    )
    .await;
    let revision_after_schema = select_revision(&after_schema, latest_revision_id);
    assert_eq!(
        revision_after_schema["gate_summary"]["schema_status"].as_str(),
        Some("passed")
    );
    let schema_snapshot = find_snapshot(revision_after_schema, "schema").unwrap();
    assert_eq!(schema_snapshot["status"].as_str(), Some("passed"));
    revision_version = revision_after_schema["revision"]["version"]
        .as_i64()
        .unwrap();

    let policy_payload = json!({
        "policy_status": "vetoed",
        "veto_reasons": ["policy_hook:remediation_gate=pending-signal"],
        "gate_context": {"policy": "trust-intelligence"},
        "metadata": {"ticket": "RISK-42"},
        "expected_revision_version": revision_version,
    });

    let after_policy = apply_workspace_policy(
        &app,
        &token,
        workspace_id,
        latest_revision_id,
        policy_payload,
    )
    .await;
    let revision_after_policy = select_revision(&after_policy, latest_revision_id);
    assert_eq!(
        revision_after_policy["gate_summary"]["policy_status"].as_str(),
        Some("vetoed")
    );
    assert!(revision_after_policy["gate_summary"]["policy_veto_reasons"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| {
            value
                .as_str()
                .map(|entry| entry.starts_with("policy_hook:remediation_gate"))
                .unwrap_or(false)
        }));
    let policy_snapshot = find_snapshot(revision_after_policy, "policy").unwrap();
    assert_eq!(policy_snapshot["status"].as_str(), Some("vetoed"));
    revision_version = revision_after_policy["revision"]["version"]
        .as_i64()
        .unwrap();
    let stale_promotion_revision_version = revision_version;

    let simulation_payload = json!({
        "simulator_kind": "chaos-matrix",
        "execution_state": "succeeded",
        "gate_context": {"scenario": "workspace-lifecycle"},
        "diff_snapshot": {"changes": 3},
        "metadata": {"transcript": "ok"},
        "expected_revision_version": revision_version,
    });

    let after_simulation = apply_workspace_simulation(
        &app,
        &token,
        workspace_id,
        latest_revision_id,
        simulation_payload,
    )
    .await;
    let revision_after_sim = select_revision(&after_simulation, latest_revision_id);
    assert_eq!(
        revision_after_sim["gate_summary"]["simulation_status"].as_str(),
        Some("succeeded")
    );
    assert!(revision_after_sim["sandbox_executions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| {
            entry["simulator_kind"].as_str() == Some("chaos-matrix")
                && entry["execution_state"].as_str() == Some("succeeded")
        }));
    revision_version = revision_after_sim["revision"]["version"].as_i64().unwrap();

    let stale_promotion = post_workspace_request(
        &app,
        &token,
        format!(
            "/api/trust/remediation/workspaces/{workspace_id}/revisions/{latest_revision_id}/promotion"
        ),
        json!({
            "promotion_status": "completed",
            "notes": ["ready"],
            "expected_workspace_version": workspace_version,
            "expected_revision_version": stale_promotion_revision_version,
        }),
    )
    .await;
    assert_eq!(stale_promotion.status(), StatusCode::CONFLICT);

    let promotion_payload = json!({
        "promotion_status": "completed",
        "notes": ["validated via harness"],
        "gate_context": {"lane": "alpha", "stage": "production"},
        "expected_workspace_version": workspace_version,
        "expected_revision_version": revision_version,
    });

    let after_promotion = apply_workspace_promotion(
        &app,
        &token,
        workspace_id,
        latest_revision_id,
        promotion_payload,
    )
    .await;
    let promotion_runs = after_promotion["promotion_runs"].as_array().unwrap();
    assert!(
        promotion_runs
            .iter()
            .any(|run| run["runtime_vm_instance_id"].as_i64() == Some(vm_instance_id)),
        "promotion response should include staged automation run"
    );
    let staged_run = promotion_runs
        .iter()
        .find(|run| run["runtime_vm_instance_id"].as_i64() == Some(vm_instance_id))
        .cloned()
        .unwrap();
    assert_eq!(
        after_promotion["workspace"]["lifecycle_state"].as_str(),
        Some("promoted")
    );
    let promoted_revision = select_revision(&after_promotion, latest_revision_id);
    assert_eq!(
        promoted_revision["gate_summary"]["promotion_status"].as_str(),
        Some("completed")
    );
    let promotion_snapshot = find_snapshot(promoted_revision, "promotion").unwrap();
    assert_eq!(promotion_snapshot["status"].as_str(), Some("completed"));
    assert!(promotion_snapshot["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| {
            note.as_str()
                .map(|entry| entry.starts_with("requested_by="))
                .unwrap_or(false)
        }));
    tokio::time::sleep(StdDuration::from_millis(50)).await;
    let automation_runs = list_workspace_runs(&app, &token, workspace_id, latest_revision_id).await;
    assert!(automation_runs.iter().any(|run| {
        run["workspace_revision_id"].as_i64() == Some(latest_revision_id)
            && run["runtime_vm_instance_id"].as_i64() == Some(vm_instance_id)
    }));
    let automation_run = automation_runs
        .into_iter()
        .find(|run| run["runtime_vm_instance_id"].as_i64() == Some(vm_instance_id))
        .unwrap();
    assert_eq!(staged_run["id"], automation_run["id"]);
    assert_eq!(
        staged_run["automation_payload"],
        automation_run["automation_payload"]
    );
    assert_eq!(
        staged_run["promotion_gate_context"],
        automation_run["promotion_gate_context"]
    );
    assert_eq!(automation_run["workspace_id"].as_i64(), Some(workspace_id));
    assert_eq!(
        automation_run["workspace_revision_id"].as_i64(),
        Some(latest_revision_id)
    );
    let gate_context = automation_run["promotion_gate_context"]
        .as_object()
        .unwrap();
    assert_eq!(
        gate_context.get("lane").and_then(|value| value.as_str()),
        Some("alpha")
    );
    let promotion_metadata = automation_run["metadata"]["promotion"].as_object().unwrap();
    assert!(promotion_metadata["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note.as_str() == Some("validated via harness")));
    let promoted_workspace_version = after_promotion["workspace"]["version"].as_i64().unwrap();
    assert_eq!(promoted_workspace_version, workspace_version + 1);
    let listed = list_workspaces(&app, &token).await;
    assert!(listed.iter().any(|entry| {
        entry["workspace"]["id"].as_i64() == Some(workspace_id)
            && entry["workspace"]["lifecycle_state"].as_str() == Some("promoted")
    }));

    let fetched = fetch_workspace_details(&app, &token, workspace_id).await;
    let fetched_revision = select_revision(&fetched, latest_revision_id);
    assert_eq!(
        fetched_revision["gate_summary"]["promotion_status"].as_str(),
        Some("completed")
    );
    assert_eq!(
        fetched_revision["gate_summary"]["policy_veto_reasons"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(fetched_revision["gate_summary"]["policy_veto_reasons"][0]
        .as_str()
        .unwrap()
        .starts_with("policy_hook:remediation_gate"));

    let initial_revision_after_promotion = select_revision(
        &fetched,
        initial_revision["revision"]["id"].as_i64().unwrap(),
    );
    assert_eq!(
        initial_revision_after_promotion["revision"]["version"].as_i64(),
        Some(initial_revision_version)
    );
}

#[sqlx::test]
#[ignore = "requires DATABASE_URL with Postgres server"]
async fn remediation_workspace_promotion_multiple_targets(pool: PgPool) {
    let harness = bootstrap_remediation_harness(&pool).await;
    let app = harness.app.clone();
    let token = harness.token.clone();

    let secondary_vm: i64 = sqlx::query_scalar(
        "INSERT INTO runtime_vm_instances (server_id, instance_id) VALUES ($1, $2) RETURNING id",
    )
    .bind(harness.server_id)
    .bind("vm-remediation-2")
    .fetch_one(&harness.pool)
    .await
    .unwrap();

    let restart_playbook = json!({
        "playbook_key": "vm.restart",
        "display_name": "VM Restart",
        "description": "Restart VM instance",
        "executor_type": "shell",
        "approval_required": false,
        "metadata": {"origin": "multi-target"},
    });
    create_playbook(&app, &token, restart_playbook).await;

    let redeploy_playbook = json!({
        "playbook_key": "vm.redeploy",
        "display_name": "VM Redeploy",
        "description": "Redeploy VM instance",
        "executor_type": "shell",
        "approval_required": false,
        "metadata": {"origin": "multi-target"},
    });
    create_playbook(&app, &token, redeploy_playbook).await;

    let workspace_payload = json!({
        "workspace_key": "workspace.multi",
        "display_name": "Workspace Multi Target",
        "description": "Covers nested targets and default playbooks",
        "plan": {
            "playbooks": ["vm.restart"],
            "targets": {
                "lanes": [
                    {
                        "lane": "cli",
                        "stage": "promotion",
                        "targets": [
                            {
                                "instance_id": secondary_vm.to_string(),
                                "automation_payload": {"kind": "lane"}
                            }
                        ]
                    }
                ],
                "direct": [
                    {
                        "runtime_vm_instance_id": harness.vm_instance_id,
                        "playbook": "vm.redeploy",
                        "automation_payload": {"kind": "direct"}
                    }
                ]
            }
        },
        "metadata": {"origin": "integration"},
        "lineage_tags": ["validation:remediation-workspace-multi"],
        "lineage_labels": ["channel:multi"],
    });

    let workspace = create_workspace(&app, &token, workspace_payload).await;
    let workspace_id = workspace["workspace"]["id"].as_i64().unwrap();
    let mut workspace_version = workspace["workspace"]["version"].as_i64().unwrap();
    let revision_id = workspace["workspace"]["active_revision_id"]
        .as_i64()
        .unwrap();
    let revision = select_revision(&workspace, revision_id);
    let mut revision_version = revision["revision"]["version"].as_i64().unwrap();

    let schema_envelope = apply_workspace_schema(
        &app,
        &token,
        workspace_id,
        revision_id,
        json!({
            "result_status": "passed",
            "gate_context": {"validator": "multi"},
            "metadata": {"notes": "schema-ok"},
            "expected_workspace_version": workspace_version,
            "expected_revision_version": revision_version,
        }),
    )
    .await;
    workspace_version = schema_envelope["workspace"]["version"].as_i64().unwrap();
    let revision_after_schema = select_revision(&schema_envelope, revision_id);
    revision_version = revision_after_schema["revision"]["version"]
        .as_i64()
        .unwrap();

    let policy_envelope = apply_workspace_policy(
        &app,
        &token,
        workspace_id,
        revision_id,
        json!({
            "policy_status": "approved",
            "gate_context": {"policy": "multi"},
            "metadata": {"ticket": "MULTI-1"},
            "expected_workspace_version": workspace_version,
            "expected_revision_version": revision_version,
        }),
    )
    .await;
    workspace_version = policy_envelope["workspace"]["version"].as_i64().unwrap();
    let revision_after_policy = select_revision(&policy_envelope, revision_id);
    revision_version = revision_after_policy["revision"]["version"]
        .as_i64()
        .unwrap();

    let simulation_envelope = apply_workspace_simulation(
        &app,
        &token,
        workspace_id,
        revision_id,
        json!({
            "simulator_kind": "harness",
            "execution_state": "succeeded",
            "gate_context": {"simulator": "multi"},
            "metadata": {"diff": "ok"},
            "expected_workspace_version": workspace_version,
            "expected_revision_version": revision_version,
        }),
    )
    .await;
    workspace_version = simulation_envelope["workspace"]["version"]
        .as_i64()
        .unwrap();
    let revision_after_sim = select_revision(&simulation_envelope, revision_id);
    revision_version = revision_after_sim["revision"]["version"].as_i64().unwrap();

    let promotion_gate_context = json!({"lane": "cli", "stage": "promotion"});
    let promotion_envelope = apply_workspace_promotion(
        &app,
        &token,
        workspace_id,
        revision_id,
        json!({
            "promotion_status": "completed",
            "gate_context": promotion_gate_context,
            "notes": ["multi-target"],
            "expected_workspace_version": workspace_version,
            "expected_revision_version": revision_version,
        }),
    )
    .await;

    let revision_after_promotion = select_revision(&promotion_envelope, revision_id);
    assert_eq!(
        revision_after_promotion["gate_summary"]["promotion_status"].as_str(),
        Some("completed")
    );

    let primary_runs = list_runs_for_instance(&app, &token, harness.vm_instance_id).await;
    let secondary_runs = list_runs_for_instance(&app, &token, secondary_vm).await;

    assert_eq!(primary_runs.len(), 1);
    assert_eq!(secondary_runs.len(), 1);

    let primary_run = &primary_runs[0];
    assert_eq!(primary_run["run"]["playbook"].as_str(), Some("vm.redeploy"));
    assert_eq!(
        primary_run["run"]["promotion_gate_context"]["lane"].as_str(),
        Some("cli")
    );
    assert_eq!(
        primary_run["run"]["metadata"]["target"]["automation_payload"]["kind"].as_str(),
        Some("direct")
    );

    let secondary_run = &secondary_runs[0];
    assert_eq!(
        secondary_run["run"]["playbook"].as_str(),
        Some("vm.restart")
    );
    assert_eq!(
        secondary_run["run"]["promotion_gate_context"]["lane"].as_str(),
        Some("cli")
    );
    assert_eq!(
        secondary_run["run"]["metadata"]["target"]["lane"].as_str(),
        Some("cli")
    );
    assert_eq!(
        secondary_run["run"]["metadata"]["target"]["stage"].as_str(),
        Some("promotion")
    );
}

// key: validation -> remediation-workspace:pending-refresh
#[sqlx::test]
#[ignore = "requires DATABASE_URL with Postgres server"]
async fn remediation_workspace_promotion_refreshes_pending_run_payload(pool: PgPool) {
    let harness = bootstrap_remediation_harness(&pool).await;
    let app = harness.app.clone();
    let token = harness.token.clone();

    let playbook_payload = json!({
        "playbook_key": "vm.promotion.refresh",
        "display_name": "Refresh Pending Promotion Run",
        "description": "Ensures promotion refresh updates pending runs",
        "executor_type": "shell",
        "approval_required": true,
        "metadata": {"origin": "workspace-refresh"},
    });
    create_playbook(&app, &token, playbook_payload).await;

    let workspace_payload = json!({
        "workspace_key": "workspace.refresh",
        "display_name": "Workspace Promotion Refresh",
        "description": "Covers refreshing pending promotion automation payloads",
        "plan": {
            "playbooks": ["vm.promotion.refresh"],
            "targets": [{
                "runtime_vm_instance_id": harness.vm_instance_id,
                "playbook": "vm.promotion.refresh",
                "automation_payload": {"kind": "initial", "attempt": 1},
            }],
        },
        "metadata": {"channel": "refresh"},
        "lineage_tags": ["validation:remediation-workspace-refresh"],
        "lineage_labels": ["channel:refresh"],
    });
    let workspace = create_workspace(&app, &token, workspace_payload).await;
    let workspace_id = workspace["workspace"]["id"].as_i64().unwrap();
    let workspace_version = workspace["workspace"]["version"].as_i64().unwrap();
    let revision_id = workspace["workspace"]["active_revision_id"]
        .as_i64()
        .unwrap();
    let revision_envelope = select_revision(&workspace, revision_id);
    let mut revision_version = revision_envelope["revision"]["version"].as_i64().unwrap();

    let schema_envelope = apply_workspace_schema(
        &app,
        &token,
        workspace_id,
        revision_id,
        json!({
            "result_status": "passed",
            "errors": [],
            "gate_context": {"stage": "schema"},
            "metadata": {"validator": "workspace-refresh"},
            "expected_revision_version": revision_version,
        }),
    )
    .await;
    let schema_revision = select_revision(&schema_envelope, revision_id);
    revision_version = schema_revision["revision"]["version"].as_i64().unwrap();

    let policy_expected_workspace_version = workspace_version;
    let policy_envelope = apply_workspace_policy(
        &app,
        &token,
        workspace_id,
        revision_id,
        json!({
            "policy_status": "approved",
            "veto_reasons": [],
            "gate_context": {"stage": "policy"},
            "metadata": {"ticket": "REFRESH-1"},
            "expected_workspace_version": policy_expected_workspace_version,
            "expected_revision_version": revision_version,
        }),
    )
    .await;
    let policy_workspace_version = policy_envelope["workspace"]["version"].as_i64().unwrap();
    let policy_revision = select_revision(&policy_envelope, revision_id);
    revision_version = policy_revision["revision"]["version"].as_i64().unwrap();

    let simulation_expected_workspace_version = policy_workspace_version;
    let simulation_envelope = apply_workspace_simulation(
        &app,
        &token,
        workspace_id,
        revision_id,
        json!({
            "simulator_kind": "workspace-refresh",
            "execution_state": "succeeded",
            "gate_context": {"stage": "simulation"},
            "metadata": {"diff": "clean"},
            "expected_workspace_version": simulation_expected_workspace_version,
            "expected_revision_version": revision_version,
        }),
    )
    .await;
    let simulation_workspace_version = simulation_envelope["workspace"]["version"]
        .as_i64()
        .unwrap();
    let simulation_revision = select_revision(&simulation_envelope, revision_id);
    revision_version = simulation_revision["revision"]["version"].as_i64().unwrap();

    let first_gate_context = json!({"lane": "initial", "stage": "promotion"});
    let promotion_expected_workspace_version = simulation_workspace_version;
    let first_envelope = apply_workspace_promotion(
        &app,
        &token,
        workspace_id,
        revision_id,
        json!({
            "promotion_status": "approved",
            "gate_context": first_gate_context,
            "notes": ["initial"],
            "expected_workspace_version": promotion_expected_workspace_version,
            "expected_revision_version": revision_version,
        }),
    )
    .await;

    let first_promotion_runs = first_envelope["promotion_runs"].as_array().unwrap();
    assert_eq!(first_promotion_runs.len(), 1);
    assert_eq!(
        first_promotion_runs[0]["automation_payload"]["attempt"].as_i64(),
        Some(1)
    );
    assert_eq!(
        first_promotion_runs[0]["promotion_gate_context"]["lane"].as_str(),
        Some("initial")
    );

    let first_runs = list_runs_for_instance(&app, &token, harness.vm_instance_id).await;
    assert_eq!(first_runs.len(), 1, "expected initial promotion run");
    let first_run = &first_runs[0]["run"];
    let run_id = first_run["id"].as_i64().unwrap();
    assert_eq!(
        first_run["automation_payload"]["attempt"].as_i64(),
        Some(1),
        "initial automation payload should be recorded",
    );

    let first_workspace_version = first_envelope["workspace"]["version"].as_i64().unwrap();

    let refreshed_plan = json!({
        "playbooks": ["vm.promotion.refresh"],
        "targets": [{
            "runtime_vm_instance_id": harness.vm_instance_id,
            "playbook": "vm.promotion.refresh",
            "automation_payload": {"kind": "refreshed", "attempt": 2},
        }],
    });
    let refreshed_revision_version: i64 = sqlx::query_scalar(
        "UPDATE runtime_vm_remediation_workspace_revisions \
         SET plan = $1, version = version + 1 \
         WHERE id = $2 RETURNING version",
    )
    .bind(refreshed_plan)
    .bind(revision_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    let refreshed_gate_context = json!({"lane": "refresh", "stage": "promotion"});
    let refreshed_envelope = apply_workspace_promotion(
        &app,
        &token,
        workspace_id,
        revision_id,
        json!({
            "promotion_status": "approved",
            "gate_context": refreshed_gate_context,
            "notes": ["refreshed"],
            "expected_workspace_version": first_workspace_version,
            "expected_revision_version": refreshed_revision_version,
        }),
    )
    .await;

    let refreshed_promotion_runs = refreshed_envelope["promotion_runs"].as_array().unwrap();
    assert_eq!(refreshed_promotion_runs.len(), 1);
    assert_eq!(
        refreshed_promotion_runs[0]["automation_payload"]["attempt"].as_i64(),
        Some(2)
    );
    assert_eq!(
        refreshed_promotion_runs[0]["promotion_gate_context"]["lane"].as_str(),
        Some("refresh")
    );

    let refreshed_runs = list_runs_for_instance(&app, &token, harness.vm_instance_id).await;
    assert_eq!(
        refreshed_runs.len(),
        1,
        "promotion refresh should reuse run"
    );
    let refreshed_run = &refreshed_runs[0]["run"];
    assert_eq!(refreshed_run["id"].as_i64(), Some(run_id));
    assert_eq!(
        refreshed_run["automation_payload"]["attempt"].as_i64(),
        Some(2),
        "automation payload should reflect refreshed target",
    );
    assert_eq!(
        refreshed_run["automation_payload"]["kind"].as_str(),
        Some("refreshed"),
    );
    assert_eq!(
        refreshed_run["promotion_gate_context"]["lane"].as_str(),
        Some("refresh"),
        "promotion gate context should update during refresh",
    );
    assert_eq!(
        refreshed_run["metadata"]["target"]["automation_payload"]["attempt"].as_i64(),
        Some(2),
    );

    let previous_metadata = refreshed_run["metadata"]["previous_metadata"].clone();
    assert_eq!(
        previous_metadata["target"]["automation_payload"]["attempt"].as_i64(),
        Some(1),
        "previous metadata should capture initial promotion payload",
    );
    assert_eq!(
        refreshed_run["metadata"]["promotion"]["notes"]
            .as_array()
            .and_then(|notes| notes.first())
            .and_then(|value| value.as_str()),
        Some("refreshed"),
    );
}

// key: validation -> remediation-stream:workspace-context
#[sqlx::test]
#[ignore = "requires DATABASE_URL with Postgres server"]
async fn remediation_workspace_promotion_stream_includes_workspace_context(pool: PgPool) {
    let harness = bootstrap_remediation_harness(&pool).await;
    let app = harness.app.clone();
    let token = harness.token.clone();

    let playbook_payload = json!({
        "playbook_key": "vm.workspace.sse",
        "display_name": "Workspace SSE Playbook",
        "description": "Ensures SSE payload carries workspace linkage",
        "executor_type": "shell",
        "approval_required": true,
        "metadata": {"origin": "workspace-sse"},
    });
    create_playbook(&app, &token, playbook_payload).await;

    let workspace_payload = json!({
        "workspace_key": "workspace.sse",
        "display_name": "Workspace SSE Coverage",
        "description": "Covers SSE workspace context propagation",
        "plan": {
            "playbooks": ["vm.workspace.sse"],
            "targets": [
                {
                    "runtime_vm_instance_id": harness.vm_instance_id,
                    "playbook": "vm.workspace.sse",
                    "automation_payload": {"scenario": "workspace-sse"},
                }
            ],
        },
        "metadata": {"channel": "sse"},
        "lineage_tags": ["coverage"],
        "lineage_labels": ["channel:sse"],
    });
    let workspace = create_workspace(&app, &token, workspace_payload).await;
    let workspace_id = workspace["workspace"]["id"].as_i64().unwrap();
    let mut workspace_version = workspace["workspace"]["version"].as_i64().unwrap();
    let revision_id = workspace["workspace"]["active_revision_id"]
        .as_i64()
        .unwrap();
    let revision_envelope = select_revision(&workspace, revision_id);
    let mut revision_version = revision_envelope["revision"]["version"].as_i64().unwrap();

    let schema_envelope = apply_workspace_schema(
        &app,
        &token,
        workspace_id,
        revision_id,
        json!({
            "result_status": "passed",
            "errors": [],
            "gate_context": {"stage": "schema"},
            "metadata": {"validator": "workspace-sse"},
            "expected_revision_version": revision_version,
        }),
    )
    .await;
    let schema_revision = select_revision(&schema_envelope, revision_id);
    revision_version = schema_revision["revision"]["version"].as_i64().unwrap();

    let policy_envelope = apply_workspace_policy(
        &app,
        &token,
        workspace_id,
        revision_id,
        json!({
            "policy_status": "approved",
            "veto_reasons": [],
            "gate_context": {"stage": "policy"},
            "metadata": {"ticket": "SSE-1"},
            "expected_workspace_version": workspace_version,
            "expected_revision_version": revision_version,
        }),
    )
    .await;
    workspace_version = policy_envelope["workspace"]["version"].as_i64().unwrap();
    let policy_revision = select_revision(&policy_envelope, revision_id);
    revision_version = policy_revision["revision"]["version"].as_i64().unwrap();

    let simulation_envelope = apply_workspace_simulation(
        &app,
        &token,
        workspace_id,
        revision_id,
        json!({
            "simulator_kind": "workspace-sse",
            "execution_state": "succeeded",
            "gate_context": {"stage": "simulation"},
            "metadata": {"diff": "clean"},
            "expected_workspace_version": workspace_version,
            "expected_revision_version": revision_version,
        }),
    )
    .await;
    workspace_version = simulation_envelope["workspace"]["version"]
        .as_i64()
        .unwrap();
    let simulation_revision = select_revision(&simulation_envelope, revision_id);
    revision_version = simulation_revision["revision"]["version"].as_i64().unwrap();

    let promotion_gate_context = json!({"lane": "sse", "stage": "promotion"});
    let _promotion_envelope = apply_workspace_promotion(
        &app,
        &token,
        workspace_id,
        revision_id,
        json!({
            "promotion_status": "completed",
            "gate_context": promotion_gate_context,
            "notes": ["workspace-sse"],
            "expected_workspace_version": workspace_version,
            "expected_revision_version": revision_version,
        }),
    )
    .await;

    tokio::time::sleep(StdDuration::from_millis(50)).await;

    let runs = list_workspace_runs(&app, &token, workspace_id, revision_id).await;
    assert_eq!(runs.len(), 1, "expected single promotion-triggered run");
    let run = runs.into_iter().next().unwrap();
    let run_id = run["id"].as_i64().unwrap();
    let run_version = run["version"].as_i64().unwrap();

    let stream_task = tokio::spawn(collect_stream_events(app.clone(), token.clone(), run_id));

    let approval_payload = json!({
        "new_state": "approved",
        "expected_version": run_version,
    });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/api/trust/remediation/runs/{run_id}/approval"))
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::from(approval_payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let events = stream_task
        .await
        .expect("stream collection failed for workspace remediation run");
    assert!(
        !events.is_empty(),
        "expected remediation SSE events for workspace-linked run"
    );

    let first = events.first().unwrap();
    assert_eq!(
        first.get("workspace_id").and_then(Value::as_i64),
        Some(workspace_id)
    );
    assert_eq!(
        first.get("workspace_revision_id").and_then(Value::as_i64),
        Some(revision_id)
    );
    assert_eq!(
        first
            .get("promotion_gate_context")
            .and_then(|value| value.get("lane"))
            .and_then(Value::as_str),
        Some("sse")
    );
    assert_eq!(
        first
            .get("automation_payload")
            .and_then(Value::as_object)
            .and_then(|payload| payload.get("scenario"))
            .and_then(Value::as_str),
        Some("workspace-sse")
    );
}

async fn create_playbook(
    app: &Router,
    token: &str,
    payload: serde_json::Value,
) -> serde_json::Value {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/trust/remediation/playbooks")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = body::to_bytes(response.into_body()).await.unwrap();
    serde_json::from_slice(&body_bytes).unwrap()
}

async fn enqueue_run(app: &Router, token: &str, payload: serde_json::Value) -> serde_json::Value {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/trust/remediation/runs")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = body::to_bytes(response.into_body()).await.unwrap();
    serde_json::from_slice(&body_bytes).unwrap()
}

async fn list_runs_for_instance(
    app: &Router,
    token: &str,
    runtime_vm_instance_id: i64,
) -> Vec<serde_json::Value> {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!(
                    "/api/trust/remediation/runs?runtime_vm_instance_id={}",
                    runtime_vm_instance_id
                ))
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = body::to_bytes(response.into_body()).await.unwrap();
    serde_json::from_slice(&body_bytes).unwrap()
}

async fn post_json(app: &Router, token: &str, uri: &str, payload: String) -> Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(uri)
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::from(payload))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn collect_stream_events(app: Router, token: String, run_id: i64) -> Vec<Value> {
    let uri = format!("/api/trust/remediation/stream?run_id={run_id}");
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(uri)
                .header("Authorization", format!("Bearer {}", token))
                .header("Accept", "text/event-stream")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body();
    timeout(StdDuration::from_secs(30), read_sse_stream(body, run_id))
        .await
        .expect("timed out waiting for remediation SSE")
}

async fn read_sse_stream<B>(mut body: B, run_id: i64) -> Vec<Value>
where
    B: HttpBody<Data = Bytes, Error = axum::Error> + Unpin,
{
    let mut buffer = String::new();
    let mut events = Vec::new();

    while let Some(chunk) = body.data().await {
        let chunk = chunk.expect("failed to read SSE chunk");
        buffer.push_str(std::str::from_utf8(&chunk).expect("SSE chunk not UTF-8"));

        loop {
            let Some(index) = buffer.find("\n\n") else {
                break;
            };
            let frame = buffer[..index].to_string();
            buffer.drain(..index + 2);

            for line in frame.lines() {
                if let Some(data) = line.strip_prefix("data:") {
                    let payload = data.trim();
                    if payload.is_empty() {
                        continue;
                    }
                    let value: Value = serde_json::from_str(payload)
                        .expect("failed to deserialize SSE remediation payload");
                    if value
                        .get("run_id")
                        .and_then(|entry| entry.as_i64())
                        .filter(|current| *current == run_id)
                        .is_none()
                    {
                        continue;
                    }
                    events.push(value.clone());
                    let is_terminal = value
                        .get("event")
                        .and_then(|entry| entry.get("event"))
                        .and_then(|entry| entry.as_str())
                        .map(|kind| kind == "status")
                        .unwrap_or(false)
                        && value
                            .get("event")
                            .and_then(|entry| entry.get("status"))
                            .and_then(|entry| entry.as_str())
                            .map(|status| status == "completed" || status == "failed")
                            .unwrap_or(false);
                    if is_terminal {
                        return events;
                    }
                }
            }
        }
    }

    events
}

// key: validation -> remediation-lifecycle-harness
#[sqlx::test]
#[ignore = "requires DATABASE_URL with Postgres server"]
async fn remediation_lifecycle_harness(pool: PgPool) {
    let harness = bootstrap_remediation_harness(&pool).await;
    let app = harness.app.clone();
    let token = harness.token.clone();
    let operator_id = harness.operator_id;
    let server_id = harness.server_id;
    let vm_instance_id = harness.vm_instance_id;

    let playbook_payload = json!({
        "playbook_key": "vm.restart",
        "display_name": "Restart VM",
        "description": "Restart the workload",
        "executor_type": "shell",
        "approval_required": true,
        "sla_duration_seconds": 600,
        "metadata": {"tier": "gold"}
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/trust/remediation/playbooks")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::from(playbook_payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = body::to_bytes(response.into_body()).await.unwrap();
    let mut playbook: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(playbook["playbook_key"], "vm.restart");
    assert_eq!(playbook["approval_required"], true);

    let playbook_id = playbook["id"].as_i64().unwrap();
    let current_version = playbook["version"].as_i64().unwrap();

    let update_payload = json!({
        "display_name": "Restart VM Safely",
        "metadata": {"tier": "gold", "caution": true},
        "expected_version": current_version
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PATCH)
                .uri(format!("/api/trust/remediation/playbooks/{}", playbook_id))
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::from(update_payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = body::to_bytes(response.into_body()).await.unwrap();
    playbook = serde_json::from_slice(&body_bytes).unwrap();
    let updated_version = playbook["version"].as_i64().unwrap();
    assert!(updated_version > current_version);

    let stale_update = json!({
        "description": "Out of date",
        "expected_version": current_version
    });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PATCH)
                .uri(format!("/api/trust/remediation/playbooks/{}", playbook_id))
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::from(stale_update.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);

    let run_request = json!({
        "runtime_vm_instance_id": vm_instance_id,
        "playbook": "vm.restart",
        "metadata": {"reason": "integration"},
        "automation_payload": null
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/trust/remediation/runs")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::from(run_request.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = body::to_bytes(response.into_body()).await.unwrap();
    let run_response: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(run_response["created"], true);
    let run = run_response["run"].clone();
    let run_id = run["id"].as_i64().unwrap();
    let run_version = run["version"].as_i64().unwrap();
    assert_eq!(run["approval_state"], "pending");

    let duplicate_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/trust/remediation/runs")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::from(run_request.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(duplicate_response.status(), StatusCode::CONFLICT);

    let registry_state = upsert_state(
        &pool,
        UpsertRuntimeVmTrustRegistryState {
            runtime_vm_instance_id: vm_instance_id,
            attestation_status: "untrusted",
            lifecycle_state: "remediating",
            remediation_state: Some("remediation:pending-approval"),
            remediation_attempts: 1,
            freshness_deadline: None,
            provenance_ref: None,
            provenance: None,
            expected_version: None,
        },
    )
    .await
    .unwrap();

    let gate = evaluate_placement_gate(&pool, server_id)
        .await
        .unwrap()
        .unwrap();
    assert!(gate.blocked);
    assert!(gate
        .notes
        .iter()
        .any(|note| note.starts_with("trust:lifecycle:")));
    assert!(gate
        .notes
        .iter()
        .any(|note| note.starts_with("remediation:pending-approval")));

    let approval_payload = json!({
        "new_state": "approved",
        "approval_notes": "auto-approved",
        "expected_version": run_version
    });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/api/trust/remediation/runs/{}/approval", run_id))
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::from(approval_payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = body::to_bytes(response.into_body()).await.unwrap();
    let approved_run: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(approved_run["approval_state"], "approved");

    insert_artifact(
        &pool,
        run_id,
        "log",
        None,
        &json!({"message": "executor started"}),
        Some(operator_id),
    )
    .await
    .unwrap();

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/api/trust/remediation/runs/{}/artifacts", run_id))
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = body::to_bytes(response.into_body()).await.unwrap();
    let artifacts: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(artifacts.as_array().unwrap().len(), 1);

    let _final_state = upsert_state(
        &pool,
        UpsertRuntimeVmTrustRegistryState {
            runtime_vm_instance_id: vm_instance_id,
            attestation_status: "trusted",
            lifecycle_state: "restored",
            remediation_state: Some("remediation:automation-complete"),
            remediation_attempts: registry_state.remediation_attempts,
            freshness_deadline: None,
            provenance_ref: None,
            provenance: None,
            expected_version: Some(registry_state.version),
        },
    )
    .await
    .unwrap();

    let gate = evaluate_placement_gate(&pool, server_id)
        .await
        .unwrap()
        .unwrap();
    assert!(!gate.blocked);
    assert!(gate
        .notes
        .iter()
        .any(|note| note.starts_with("trust:lifecycle:")));
}

// key: validation -> remediation-concurrency
#[sqlx::test]
#[ignore = "requires DATABASE_URL with Postgres server"]
async fn remediation_concurrent_enqueue_dedupe(pool: PgPool) {
    let harness = bootstrap_remediation_harness(&pool).await;
    let app = harness.app.clone();
    let token = harness.token.clone();
    let vm_instance_id = harness.vm_instance_id;

    let playbook_payload = json!({
        "playbook_key": "vm.restart",
        "display_name": "Restart VM",
        "description": "Restart the workload",
        "executor_type": "shell",
        "approval_required": true,
        "sla_duration_seconds": 600,
        "metadata": {"tier": "gold"}
    });

    let create_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/trust/remediation/playbooks")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::from(playbook_payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_response.status(), StatusCode::OK);

    let run_request = json!({
        "runtime_vm_instance_id": vm_instance_id,
        "playbook": "vm.restart",
        "metadata": {"reason": "concurrency"},
        "automation_payload": null
    });
    let run_payload = run_request.to_string();
    let app_first = app.clone();
    let app_second = app.clone();
    let token_first = token.clone();
    let token_second = token.clone();
    let payload_first = run_payload.clone();
    let payload_second = run_payload;

    let (resp_a, resp_b) = tokio::join!(
        async move {
            app_first
                .oneshot(
                    Request::builder()
                        .method(Method::POST)
                        .uri("/api/trust/remediation/runs")
                        .header("Content-Type", "application/json")
                        .header("Authorization", format!("Bearer {}", token_first))
                        .body(Body::from(payload_first))
                        .unwrap(),
                )
                .await
                .unwrap()
        },
        async move {
            app_second
                .oneshot(
                    Request::builder()
                        .method(Method::POST)
                        .uri("/api/trust/remediation/runs")
                        .header("Content-Type", "application/json")
                        .header("Authorization", format!("Bearer {}", token_second))
                        .body(Body::from(payload_second))
                        .unwrap(),
                )
                .await
                .unwrap()
        },
    );

    let (ok_response, conflict_response) = if resp_a.status() == StatusCode::OK {
        (resp_a, resp_b)
    } else {
        (resp_b, resp_a)
    };

    assert_eq!(ok_response.status(), StatusCode::OK);
    assert_eq!(conflict_response.status(), StatusCode::CONFLICT);

    let body_bytes = body::to_bytes(ok_response.into_body()).await.unwrap();
    let run_response: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(run_response["created"], true);
    let run_id = run_response["run"]["id"].as_i64().unwrap();

    let list_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/trust/remediation/runs")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list_response.status(), StatusCode::OK);
    let list_bytes = body::to_bytes(list_response.into_body()).await.unwrap();
    let runs: Value = serde_json::from_slice(&list_bytes).unwrap();
    let runs_array = runs.as_array().unwrap();
    assert_eq!(runs_array.len(), 1);
    assert_eq!(runs_array[0]["id"].as_i64().unwrap(), run_id);
}

// key: validation -> remediation-stream:sse-ordering
#[sqlx::test]
#[ignore = "requires DATABASE_URL with Postgres server"]
async fn remediation_stream_captures_manifest_metadata(pool: PgPool) {
    let harness = bootstrap_remediation_harness(&pool).await;
    let app = harness.app.clone();
    let token = harness.token.clone();

    let manifest_root = resolve_manifest_root();
    let scenarios = load_manifest_directory(&manifest_root)
        .expect("scenario manifests should be provisioned for SSE validation");
    let scenario_execution = scenarios
        .into_iter()
        .next()
        .expect("at least one scenario execution must be available");

    let scenario = scenario_execution.definition;
    let tenant = scenario_execution.tenant;
    let scenario_tag = format!("{}::{}", scenario.tag, tenant);
    let playbook_key = format!("remediation.stream.{}.{}", scenario.tag, tenant);

    let playbook_payload = json!({
        "playbook_key": playbook_key,
        "display_name": format!("{} stream validation", scenario.name),
        "description": "Validate remediation SSE stream ordering and manifest tags",
        "executor_type": "shell",
        "approval_required": true,
        "sla_duration_seconds": 120,
        "metadata": {"scenario": scenario_tag.clone(), "harness": "sse-validation"}
    });
    let playbook = create_playbook(&app, &token, playbook_payload).await;
    let playbook_id = playbook["id"].as_i64().unwrap();

    let mut metadata = json!({
        "scenario": scenario_tag.clone(),
        "manifest_tag": scenario.tag,
        "tenant": tenant,
        "playbook_id": playbook_id
    });
    merge_metadata_fields(&mut metadata, &scenario.metadata);
    let run_request = json!({
        "runtime_vm_instance_id": harness.vm_instance_id,
        "playbook": playbook_key,
        "metadata": metadata,
        "automation_payload": {
            "scenario": "sse-validation",
            "origin": "chaos-fabric"
        }
    });
    let run_response = enqueue_run(&app, &token, run_request).await;
    assert_eq!(run_response["created"], true);
    let run = run_response["run"].clone();
    assert_eq!(run["approval_state"], "pending");
    let run_id = run["id"].as_i64().unwrap();
    let run_version = run["version"].as_i64().unwrap();

    let stream_task = tokio::spawn(collect_stream_events(app.clone(), token.clone(), run_id));

    let approval_payload = json!({
        "new_state": "approved",
        "approval_notes": "SSE verification harness",
        "expected_version": run_version
    });
    let approval_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/api/trust/remediation/runs/{run_id}/approval"))
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::from(approval_payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(approval_response.status(), StatusCode::OK);

    let events = stream_task.await.expect("stream collection failed");
    assert!(!events.is_empty(), "expected remediation SSE events");

    let first_kind = events
        .first()
        .and_then(|event| event.get("event"))
        .and_then(|entry| entry.get("event"))
        .and_then(|entry| entry.as_str())
        .unwrap_or_default();
    assert_eq!(first_kind, "log", "first SSE entry should be a log event");

    let last = events
        .last()
        .and_then(|event| event.get("event"))
        .expect("expected terminal remediation event");
    assert_eq!(
        last.get("event").and_then(|entry| entry.as_str()),
        Some("status")
    );
    assert_eq!(
        last.get("status").and_then(|entry| entry.as_str()),
        Some("completed")
    );

    let tag_presence = events.iter().all(|event| {
        event
            .get("manifest_tags")
            .and_then(|value| value.as_array())
            .map(|tags| {
                tags.iter()
                    .any(|entry| entry.as_str() == Some(&scenario_tag))
            })
            .unwrap_or(false)
    });
    assert!(
        tag_presence,
        "expected manifest tags to include chaos scenario tag"
    );

    let log_events: Vec<&Value> = events
        .iter()
        .filter(|event| {
            event
                .get("event")
                .and_then(|entry| entry.get("event"))
                .and_then(|entry| entry.as_str())
                == Some("log")
        })
        .collect();
    assert!(
        !log_events.is_empty(),
        "expected remediation log events to be streamed"
    );

    let mut tick_values = Vec::new();
    for event in &log_events {
        if let Some(message) = event
            .get("event")
            .and_then(|entry| entry.get("message"))
            .and_then(|entry| entry.as_str())
        {
            if let Some(position) = message.find("tick ") {
                let remainder = &message[position + 5..];
                if let Some(number) = remainder.split_whitespace().next() {
                    if let Ok(value) = number.parse::<i32>() {
                        tick_values.push(value);
                    }
                }
            }
        }
    }
    if tick_values.len() > 1 {
        let mut sorted = tick_values.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(tick_values, sorted, "log tick ordering should be monotonic");
    }

    let metadata_event = log_events.iter().find(|event| {
        event
            .get("event")
            .and_then(|entry| entry.get("message"))
            .and_then(|entry| entry.as_str())
            .map(|message| message.contains(&scenario_tag))
            .unwrap_or(false)
    });
    assert!(
        metadata_event.is_some(),
        "expected metadata log to reference manifest tag"
    );
}

// key: validation -> remediation-stream:accelerator-policy-feedback
#[sqlx::test]
#[ignore = "requires DATABASE_URL with Postgres server"]
async fn remediation_stream_includes_policy_feedback(pool: PgPool) {
    let harness = bootstrap_remediation_harness(&pool).await;
    let manifest_root = resolve_manifest_root();
    let executions = load_manifest_directory(&manifest_root)
        .with_context(|| format!("loading scenarios from {}", manifest_root.display()))
        .unwrap();

    let accelerator_execution = executions
        .into_iter()
        .find(|execution| execution.definition.metadata.get("accelerators").is_some())
        .expect("accelerator scenario manifest should exist");

    let scenario = accelerator_execution.definition;
    let tenant = accelerator_execution.tenant;
    let scenario_tag = format!("{}::{}", scenario.tag, tenant);
    let playbook_key = format!("remediation.accelerator.{}.{}", scenario.tag, tenant);

    let mut playbook_metadata = json!({
        "scenario": scenario_tag.clone(),
        "harness": "accelerator-policy"
    });
    merge_metadata_fields(&mut playbook_metadata, &scenario.metadata);
    let playbook_payload = json!({
        "playbook_key": playbook_key,
        "display_name": format!("Accelerator validation {tenant}"),
        "description": "Accelerator remediation validation",
        "executor_type": "shell",
        "approval_required": true,
        "metadata": playbook_metadata
    });
    let playbook = create_playbook(&harness.app, &harness.token, playbook_payload).await;
    let playbook_id = playbook["id"].as_i64().unwrap();

    let mut metadata = json!({
        "scenario": scenario_tag.clone(),
        "manifest_tag": scenario.tag,
        "tenant": tenant,
        "playbook_id": playbook_id
    });
    merge_metadata_fields(&mut metadata, &scenario.metadata);

    let run_request = json!({
        "runtime_vm_instance_id": harness.vm_instance_id,
        "playbook": playbook_key,
        "metadata": metadata,
        "automation_payload": {
            "scenario": "accelerator-validation",
            "origin": "chaos-fabric"
        }
    });

    let run_response = enqueue_run(&harness.app, &harness.token, run_request).await;
    assert_eq!(run_response["created"], true);
    let run = run_response["run"].clone();
    let run_id = run["id"].as_i64().unwrap();
    let run_version = run["version"].as_i64().unwrap();

    let stream_task = tokio::spawn(collect_stream_events(
        harness.app.clone(),
        harness.token.clone(),
        run_id,
    ));

    let approval_payload = json!({
        "new_state": "approved",
        "expected_version": run_version
    });
    let approval_response = harness
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/api/trust/remediation/runs/{run_id}/approval"))
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", harness.token))
                .body(Body::from(approval_payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(approval_response.status(), StatusCode::OK);

    let events = stream_task.await.expect("stream collection failed");
    assert!(!events.is_empty(), "expected remediation SSE events");

    let status_event = events
        .iter()
        .find(|event| {
            event
                .get("event")
                .and_then(|entry| entry.get("event"))
                .and_then(|entry| entry.as_str())
                == Some("status")
        })
        .expect("status event expected");

    let policy_feedback = status_event
        .get("policy_feedback")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(policy_feedback.iter().any(|entry| entry.as_str()
        == Some("policy_hook:remediation_gate=accelerator-awaiting-attestation")));

    let policy_gate = status_event
        .get("policy_gate")
        .and_then(|value| value.as_object())
        .expect("policy gate should be present");
    let remediation_hooks = policy_gate
        .get("remediation_hooks")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(remediation_hooks.iter().any(|entry| {
        entry
            .as_str()
            .map(|value| value == "policy_hook:remediation_gate=accelerator-awaiting-attestation")
            .unwrap_or(false)
    }));

    let accelerator_gates = policy_gate
        .get("accelerator_gates")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    let gate_entry = accelerator_gates
        .iter()
        .find(|entry| {
            entry.get("accelerator_id").and_then(|value| value.as_str()) == Some("accel-lab-01")
        })
        .expect("accelerator gate expected");
    let gate_hooks = gate_entry
        .get("hooks")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(gate_hooks.iter().any(|entry| {
        entry
            .as_str()
            .map(|value| value == "policy_hook:accelerator_gate=awaiting-attestation")
            .unwrap_or(false)
    }));
    let gate_reasons = gate_entry
        .get("reasons")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(gate_reasons.iter().any(|entry| {
        entry
            .as_str()
            .map(|value| value.contains("attestation"))
            .unwrap_or(false)
    }));

    let accelerators = status_event
        .get("accelerators")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(accelerators.iter().any(|entry| {
        entry.get("accelerator_id").and_then(|value| value.as_str()) == Some("accel-lab-01")
    }));
    assert!(accelerators.iter().any(|entry| {
        entry
            .get("policy_feedback")
            .and_then(|value| value.as_array())
            .map(|feedback| {
                feedback
                    .iter()
                    .any(|item| item.as_str() == Some("accelerator:pending-remediation"))
            })
            .unwrap_or(false)
    }));
}

// key: validation -> remediation-stream:policy-veto-gates
#[sqlx::test]
#[ignore = "requires DATABASE_URL with Postgres server"]
async fn remediation_stream_exposes_policy_veto_gates(pool: PgPool) {
    let harness = bootstrap_remediation_harness(&pool).await;
    let manifest_root = resolve_manifest_root();
    let executions = load_manifest_directory(&manifest_root)
        .with_context(|| format!("loading scenarios from {}", manifest_root.display()))
        .unwrap();

    let policy_veto_execution = executions
        .into_iter()
        .find(|execution| {
            execution
                .definition
                .metadata
                .get("scenario_case")
                .and_then(|value| value.as_str())
                == Some("policy-veto")
        })
        .expect("policy veto scenario manifest should exist");

    let scenario = policy_veto_execution.definition;
    let tenant = policy_veto_execution.tenant;
    let scenario_tag = format!("{}::{}", scenario.tag, tenant);
    let playbook_key = format!("remediation.accelerator.{}.{}", scenario.tag, tenant);

    let mut playbook_metadata = json!({
        "scenario": scenario_tag.clone(),
        "harness": "accelerator-policy-veto"
    });
    merge_metadata_fields(&mut playbook_metadata, &scenario.metadata);
    let playbook_payload = json!({
        "playbook_key": playbook_key,
        "display_name": format!("Policy veto validation {tenant}"),
        "description": "Accelerator policy veto validation",
        "executor_type": "shell",
        "approval_required": true,
        "metadata": playbook_metadata
    });
    let playbook = create_playbook(&harness.app, &harness.token, playbook_payload).await;
    let playbook_id = playbook["id"].as_i64().unwrap();

    let mut metadata = json!({
        "scenario": scenario_tag.clone(),
        "manifest_tag": scenario.tag,
        "tenant": tenant,
        "playbook_id": playbook_id
    });
    merge_metadata_fields(&mut metadata, &scenario.metadata);

    let run_request = json!({
        "runtime_vm_instance_id": harness.vm_instance_id,
        "playbook": playbook_key,
        "metadata": metadata,
        "automation_payload": {
            "scenario": "accelerator-policy-veto",
            "origin": "chaos-fabric"
        }
    });

    let run_response = enqueue_run(&harness.app, &harness.token, run_request).await;
    assert_eq!(run_response["created"], true);
    let run = run_response["run"].clone();
    let run_id = run["id"].as_i64().unwrap();
    let run_version = run["version"].as_i64().unwrap();

    let stream_task = tokio::spawn(collect_stream_events(
        harness.app.clone(),
        harness.token.clone(),
        run_id,
    ));

    let approval_payload = json!({
        "new_state": "approved",
        "expected_version": run_version
    });
    let approval_response = harness
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/api/trust/remediation/runs/{run_id}/approval"))
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", harness.token))
                .body(Body::from(approval_payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(approval_response.status(), StatusCode::OK);

    let events = stream_task.await.expect("stream collection failed");
    assert!(!events.is_empty(), "expected remediation SSE events");

    let status_event = events
        .iter()
        .find(|event| {
            event
                .get("event")
                .and_then(|entry| entry.get("event"))
                .and_then(|entry| entry.as_str())
                == Some("status")
        })
        .expect("status event expected");

    let policy_gate = status_event
        .get("policy_gate")
        .and_then(|value| value.as_object())
        .expect("policy gate expected");
    let remediation_hooks = policy_gate
        .get("remediation_hooks")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(remediation_hooks.iter().any(|entry| {
        entry
            .as_str()
            .map(|value| value == "policy_hook:remediation_gate=policy-veto")
            .unwrap_or(false)
    }));
    assert!(remediation_hooks.iter().any(|entry| {
        entry
            .as_str()
            .map(|value| value == "policy_hook:remediation_gate=intelligence-block")
            .unwrap_or(false)
    }));

    let accelerator_gates = policy_gate
        .get("accelerator_gates")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    let veto_gate = accelerator_gates
        .iter()
        .find(|entry| {
            entry.get("accelerator_id").and_then(|value| value.as_str()) == Some("accel-gov-01")
        })
        .expect("accelerator gate expected");
    let veto_hooks = veto_gate
        .get("hooks")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(veto_hooks.iter().any(|entry| {
        entry
            .as_str()
            .map(|value| value == "policy_hook:accelerator_gate=vetoed")
            .unwrap_or(false)
    }));
    assert!(veto_hooks.iter().any(|entry| {
        entry
            .as_str()
            .map(|value| value == "policy_hook:intelligence_gate=anomaly-detected")
            .unwrap_or(false)
    }));
    let veto_reasons = veto_gate
        .get("reasons")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(veto_reasons.iter().any(|entry| {
        entry
            .as_str()
            .map(|value| value.contains("manual override"))
            .unwrap_or(false)
    }));

    let accelerators = status_event
        .get("accelerators")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(accelerators.iter().any(|entry| {
        entry.get("accelerator_id").and_then(|value| value.as_str()) == Some("accel-gov-01")
    }));
    assert!(accelerators.iter().any(|entry| {
        entry
            .get("policy_feedback")
            .and_then(|value| value.as_array())
            .map(|feedback| {
                feedback
                    .iter()
                    .any(|item| item.as_str() == Some("accelerator:intelligence-anomaly"))
            })
            .unwrap_or(false)
    }));
}

// key: validation -> remediation-chaos-matrix
#[sqlx::test]
#[ignore = "requires DATABASE_URL with Postgres server"]
async fn remediation_multi_tenant_chaos_matrix(pool: PgPool) {
    let harness = bootstrap_remediation_harness(&pool).await;
    let manifest_root = resolve_manifest_root();
    let executions = load_manifest_directory(&manifest_root)
        .with_context(|| format!("loading scenarios from {}", manifest_root.display()))
        .unwrap();

    let mut futures = Vec::new();
    for execution in executions {
        let harness_clone = harness.clone();
        let definition = execution.definition.clone();
        let tenant = execution.tenant.clone();
        futures.push(async move {
            run_scenario(&harness_clone, &definition, tenant.as_str()).await;
        });
    }

    join_all(futures).await;
}
