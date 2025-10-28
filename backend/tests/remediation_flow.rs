use anyhow::{bail, Context, Result};
use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
    response::Response,
    routing::{get, post},
    Extension, Router,
};
use backend::db::runtime_vm_remediation_artifacts::insert_artifact;
use backend::db::runtime_vm_remediation_runs::{mark_run_completed, mark_run_failed};
use backend::db::runtime_vm_trust_registry::{upsert_state, UpsertRuntimeVmTrustRegistryState};
use backend::policy::trust::evaluate_placement_gate;
use chrono::{Duration, Utc};
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
};
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
    let exp = (Utc::now() + Duration::hours(1)).timestamp();
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
        .layer(Extension(pool.clone()));

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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum ScenarioKind {
    TenantIsolation,
    ConcurrentApprovals,
    ExecutorOutageResumption,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct ScenarioDefinition {
    name: String,
    tag: String,
    kind: ScenarioKind,
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
    let playbook_payload = json!({
        "playbook_key": playbook_key,
        "display_name": "Executor Outage",
        "description": "Simulated executor outage",
        "executor_type": "shell",
        "approval_required": false,
        "sla_duration_seconds": 300,
        "metadata": {"scenario": scenario_tag.clone()}
    });

    create_playbook(&app, &harness.token, playbook_payload).await;
    let enqueue_payload = json!({
        "runtime_vm_instance_id": harness.vm_instance_id,
        "playbook": playbook_key,
        "metadata": {"scenario": scenario_tag.clone(), "phase": "initial"},
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

    let retry_payload = json!({
        "runtime_vm_instance_id": harness.vm_instance_id,
        "playbook": playbook_key,
        "metadata": {"scenario": scenario_tag.clone(), "phase": "retry"},
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
