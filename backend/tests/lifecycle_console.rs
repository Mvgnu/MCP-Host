use axum::{routing::get, Extension, Router};
use backend::db::runtime_vm_remediation_runs::{
    ensure_remediation_run, EnsureRemediationRunRequest,
};
use backend::db::runtime_vm_remediation_workspaces::{
    create_workspace, CreateWorkspace, WorkspaceDetails,
};
use backend::db::runtime_vm_trust_registry::{upsert_state, UpsertRuntimeVmTrustRegistryState};
use chrono::Utc;
use hyper::{body::HttpBody, Body, Request, StatusCode};
use serde_json::json;
use sqlx::PgPool;
use tower::ServiceExt;

struct LifecycleFixture {
    workspace: WorkspaceDetails,
}

async fn seed_lifecycle_fixture(pool: &PgPool) -> LifecycleFixture {
    let owner_id: i32 =
        sqlx::query_scalar("INSERT INTO users (email, password_hash) VALUES ($1, $2) RETURNING id")
            .bind("console@example.com")
            .bind("hashed")
            .fetch_one(pool)
            .await
            .unwrap();

    let server_id: i32 = sqlx::query_scalar(
        "INSERT INTO mcp_servers (owner_id, name, server_type, config, status, api_key) VALUES ($1, $2, $3, '{}'::jsonb, $4, $5) RETURNING id",
    )
    .bind(owner_id)
    .bind("console-server")
    .bind("virtual-machine")
    .bind("active")
    .bind("api-key")
    .fetch_one(pool)
    .await
    .unwrap();

    let vm_instance_id: i64 = sqlx::query_scalar(
        "INSERT INTO runtime_vm_instances (server_id, instance_id) VALUES ($1, $2) RETURNING id",
    )
    .bind(server_id)
    .bind("vm-console-1")
    .fetch_one(pool)
    .await
    .unwrap();

    let plan = json!({"steps": []});
    let metadata = json!({"targets": [{"runtime_vm_instance_id": vm_instance_id}]});

    let workspace = create_workspace(
        pool,
        CreateWorkspace {
            workspace_key: "console-workspace",
            display_name: "Lifecycle Console",
            description: Some("console snapshot"),
            owner_id,
            plan: &plan,
            metadata: Some(&metadata),
            lineage_tags: &["console"],
            lineage_labels: &["mvp"],
        },
    )
    .await
    .unwrap();

    let revision = workspace
        .revisions
        .first()
        .expect("workspace revision")
        .revision
        .clone();

    let run_metadata = json!({"workspace_id": workspace.workspace.id});
    ensure_remediation_run(
        pool,
        EnsureRemediationRunRequest {
            runtime_vm_instance_id: vm_instance_id,
            playbook_key: "shell:baseline",
            playbook_id: None,
            metadata: Some(&run_metadata),
            automation_payload: None,
            approval_required: false,
            assigned_owner_id: Some(owner_id),
            sla_duration_seconds: Some(3600),
            workspace_id: Some(workspace.workspace.id),
            workspace_revision_id: Some(revision.id),
            promotion_gate_context: None,
        },
    )
    .await
    .unwrap()
    .expect("remediation run inserted");

    upsert_state(
        pool,
        UpsertRuntimeVmTrustRegistryState {
            runtime_vm_instance_id: vm_instance_id,
            attestation_status: "trusted",
            lifecycle_state: "remediating",
            remediation_state: Some("remediation:queued"),
            remediation_attempts: 1,
            freshness_deadline: None,
            provenance_ref: Some("integration-test"),
            provenance: Some(&json!({"source": "integration"})),
            expected_version: None,
        },
    )
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO capability_intelligence_scores (server_id, capability, backend, tier, score, status, confidence, last_observed_at, notes, evidence) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, '[]'::jsonb, '[]'::jsonb)",
    )
    .bind(server_id)
    .bind("runtime-health")
    .bind(Some("remediation"))
    .bind(Some("gold"))
    .bind(85.0_f64)
    .bind("healthy")
    .bind(0.95_f64)
    .bind(Utc::now())
    .execute(pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO build_artifact_runs (server_id, source_repo, source_branch, source_revision, registry, local_image, registry_image, manifest_tag, manifest_digest, started_at, completed_at, status, multi_arch, auth_refresh_attempted, auth_refresh_succeeded, auth_rotation_attempted, auth_rotation_succeeded, credential_health_status) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)",
    )
    .bind(server_id)
    .bind(Some("https://example.com/repo"))
    .bind(Some("main"))
    .bind(Some("abc123"))
    .bind(Some("registry.local"))
    .bind("local/image:latest")
    .bind(Some("registry.local/image:latest"))
    .bind(Some("v1"))
    .bind(Some("sha256:123"))
    .bind(Utc::now())
    .bind(Utc::now())
    .bind("ready")
    .bind(true)
    .bind(false)
    .bind(false)
    .bind(false)
    .bind(false)
    .bind("healthy")
    .execute(pool)
    .await
    .unwrap();

    LifecycleFixture { workspace }
}

#[sqlx::test]
#[ignore = "requires DATABASE_URL with Postgres server"]
async fn lifecycle_console_returns_workspace_snapshot(pool: PgPool) {
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    let fixture = seed_lifecycle_fixture(&pool).await;

    let app = Router::new()
        .route(
            "/api/console/lifecycle",
            get(backend::lifecycle_console::list_snapshots),
        )
        .layer(Extension(pool.clone()));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/console/lifecycle")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    let workspaces = payload
        .get("workspaces")
        .and_then(|value| value.as_array())
        .expect("workspaces array");
    assert_eq!(workspaces.len(), 1);
    let snapshot = &workspaces[0];

    let workspace_id = snapshot
        .get("workspace")
        .and_then(|value| value.get("id"))
        .and_then(|value| value.as_i64())
        .expect("workspace id");
    assert_eq!(workspace_id, fixture.workspace.workspace.id);

    assert!(snapshot.get("active_revision").is_some());

    let runs = snapshot
        .get("recent_runs")
        .and_then(|value| value.as_array())
        .expect("recent runs");
    assert!(!runs.is_empty());
    assert!(runs.iter().any(|run| run.get("trust").is_some()));
    assert!(runs.iter().any(|run| {
        run.get("intelligence")
            .and_then(|value| value.as_array())
            .map(|entries| !entries.is_empty())
            .unwrap_or(false)
    }));
    assert!(runs.iter().any(|run| run.get("marketplace").is_some()));
}

#[sqlx::test]
#[ignore = "requires DATABASE_URL with Postgres server"]
async fn lifecycle_console_stream_emits_snapshot_event(pool: PgPool) {
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    let fixture = seed_lifecycle_fixture(&pool).await;

    let app = Router::new()
        .route(
            "/api/console/lifecycle/stream",
            get(backend::lifecycle_console::stream_snapshots),
        )
        .layer(Extension(pool.clone()));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/console/lifecycle/stream")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();
    let mut collected = Vec::new();
    if let Some(chunk) = body.data().await {
        let bytes = chunk.unwrap();
        collected.extend_from_slice(&bytes);
    }

    let payload = String::from_utf8(collected).expect("utf8");
    assert!(payload.contains("event: lifecycle-snapshot"));
    assert!(payload.contains(&fixture.workspace.workspace.id.to_string()));
}
