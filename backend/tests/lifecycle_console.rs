use axum::{routing::get, Extension, Router};
use backend::db::runtime_vm_remediation_runs::{
    ensure_remediation_run, EnsureRemediationRunRequest,
};
use backend::db::runtime_vm_remediation_workspaces::{
    create_workspace, CreateWorkspace, WorkspaceDetails,
};
use backend::db::runtime_vm_trust_registry::{upsert_state, UpsertRuntimeVmTrustRegistryState};
use chrono::{Duration, Utc};
use hyper::{body::HttpBody, Body, Request, StatusCode};
use serde_json::json;
use sqlx::PgPool;
use tower::ServiceExt;

struct LifecycleFixture {
    workspace: WorkspaceDetails,
    owner_id: i32,
    manifest_digest: String,
    run_id: i64,
    promotion_id: i64,
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
    let manifest_digest = "sha256:console-fixture".to_string();
    let metadata = json!({
        "targets": [{
            "runtime_vm_instance_id": vm_instance_id,
            "manifest_digest": manifest_digest,
            "lane": "console",
            "stage": "production"
        }]
    });

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

    let run_metadata = json!({
        "workspace_id": workspace.workspace.id,
        "target": {
            "manifest_digest": manifest_digest,
            "lane": "console",
            "stage": "production"
        },
        "promotion": {
            "manifest_digest": manifest_digest,
            "track": {"name": "Lifecycle", "tier": "gold"},
            "stage": "production"
        }
    });
    let run = ensure_remediation_run(
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

    let analytics_completed_at = run.started_at + Duration::minutes(3);
    let retry_ledger = json!([
        {
            "attempt": 1,
            "status": "failed",
            "reason": "timeout",
            "observed_at": (run.started_at + Duration::minutes(1)).to_rfc3339(),
        },
        {
            "attempt": 2,
            "status": "succeeded",
            "observed_at": analytics_completed_at.to_rfc3339(),
        },
    ]);

    sqlx::query(
        "UPDATE runtime_vm_remediation_runs SET analytics_duration_ms = $1, analytics_execution_started_at = $2, analytics_execution_completed_at = $3, analytics_retry_count = $4, analytics_retry_ledger = $5, analytics_override_actor_id = $6 WHERE id = $7",
    )
    .bind(analytics_completed_at.signed_duration_since(run.started_at).num_milliseconds())
    .bind(run.started_at)
    .bind(analytics_completed_at)
    .bind(2_i32)
    .bind(retry_ledger)
    .bind(owner_id)
    .bind(run.id)
    .execute(pool)
    .await
    .unwrap();

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

    let track_id: i32 = sqlx::query_scalar(
        "INSERT INTO promotion_tracks (owner_id, name, tier, stages, description, workflow_id) VALUES ($1, $2, $3, ARRAY['candidate','staging','production']::TEXT[], NULL, NULL) RETURNING id",
    )
    .bind(owner_id)
    .bind("Lifecycle")
    .bind("gold")
    .fetch_one(pool)
    .await
    .unwrap();

    let posture_verdict = json!({
        "allowed": false,
        "track": {"id": track_id, "name": "Lifecycle", "tier": "gold"},
        "stage": "production",
        "reasons": ["trust.lifecycle_state=quarantined"],
        "notes": ["posture:trust.lifecycle_state:quarantined"],
        "metadata": {
            "track": {"id": track_id, "name": "Lifecycle", "tier": "gold"},
            "signals": {
                "trust": {"lifecycle_state": "quarantined", "remediation_state": "remediation:active"},
                "remediation": {"status": "failed"}
            }
        }
    });

    let promotion_id: i64 = sqlx::query_scalar(
        "INSERT INTO artifact_promotions (promotion_track_id, manifest_digest, stage, status, notes, posture_verdict) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
    )
    .bind(track_id)
    .bind(&manifest_digest)
    .bind("production")
    .bind("scheduled")
    .bind(&vec!["console:seed".to_string()])
    .bind(&posture_verdict)
    .fetch_one(pool)
    .await
    .unwrap();

    sqlx::query(
        "UPDATE runtime_vm_remediation_runs SET analytics_promotion_verdict_id = $1 WHERE id = $2",
    )
    .bind(promotion_id)
    .bind(run.id)
    .execute(pool)
    .await
    .unwrap();

    LifecycleFixture {
        workspace,
        owner_id,
        manifest_digest,
        run_id: run.id,
        promotion_id,
    }
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

    let first_run = runs.first().expect("at least one run snapshot");
    assert!(first_run.get("duration_seconds").is_some());
    assert!(first_run.get("duration_ms").is_some());
    let execution_window = first_run
        .get("execution_window")
        .and_then(|value| value.as_object())
        .expect("execution window present");
    assert!(execution_window
        .get("started_at")
        .and_then(|value| value.as_str())
        .is_some());
    let retry_count = first_run
        .get("retry_count")
        .and_then(|value| value.as_i64())
        .expect("retry count available");
    assert_eq!(retry_count, 2);
    let retry_ledger = first_run
        .get("retry_ledger")
        .and_then(|value| value.as_array())
        .expect("retry ledger present");
    assert!(!retry_ledger.is_empty());
    let manual_override = first_run
        .get("manual_override")
        .and_then(|value| value.as_object())
        .expect("manual override payload");
    assert_eq!(
        manual_override
            .get("actor_email")
            .and_then(|value| value.as_str()),
        Some("console@example.com")
    );
    let artifacts = first_run
        .get("artifacts")
        .and_then(|value| value.as_array())
        .expect("artifacts array");
    assert!(artifacts.iter().any(|artifact| {
        artifact
            .get("manifest_digest")
            .and_then(|value| value.as_str())
            .map(|digest| digest == fixture.manifest_digest)
            .unwrap_or(false)
    }));
    let fingerprints = first_run
        .get("artifact_fingerprints")
        .and_then(|value| value.as_array())
        .expect("fingerprints available");
    assert!(!fingerprints.is_empty());
    let verdict = first_run
        .get("promotion_verdict")
        .and_then(|value| value.as_object())
        .expect("promotion verdict reference");
    assert_eq!(
        verdict.get("verdict_id").and_then(|value| value.as_i64()),
        Some(fixture.promotion_id)
    );

    let promotion_runs = snapshot
        .get("promotion_runs")
        .and_then(|value| value.as_array())
        .expect("promotion runs array");
    assert!(
        !promotion_runs.is_empty(),
        "expected promotion automation runs"
    );

    let promotion_postures = snapshot
        .get("promotion_postures")
        .and_then(|value| value.as_array())
        .expect("promotion postures");
    assert_eq!(promotion_postures.len(), 1);
    let promotion = &promotion_postures[0];
    assert_eq!(
        promotion.get("stage").and_then(|value| value.as_str()),
        Some("production")
    );
    assert_eq!(
        promotion
            .get("veto_reasons")
            .and_then(|value| value.as_array())
            .and_then(|values| values.get(0))
            .and_then(|value| value.as_str()),
        Some("trust.lifecycle_state=quarantined")
    );
    assert_eq!(
        promotion.get("allowed").and_then(|value| value.as_bool()),
        Some(false)
    );
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

    let data_line = payload
        .lines()
        .find(|line| line.starts_with("data: "))
        .expect("sse data line");
    let envelope: serde_json::Value =
        serde_json::from_str(data_line.trim_start_matches("data: ")).expect("json data");

    let delta = envelope
        .get("delta")
        .and_then(|value| value.get("workspaces"))
        .and_then(|value| value.as_array())
        .expect("workspace deltas");
    assert!(
        delta.iter().any(|workspace| {
            workspace
                .get("promotion_run_deltas")
                .and_then(|value| value.as_array())
                .map(|runs| !runs.is_empty())
                .unwrap_or(false)
        }),
        "expected promotion run deltas in snapshot"
    );
}
