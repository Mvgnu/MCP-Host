use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
    routing::{get, post},
    Extension, Router,
};
use backend::db::runtime_vm_remediation_artifacts::insert_artifact;
use backend::db::runtime_vm_trust_registry::{upsert_state, UpsertRuntimeVmTrustRegistryState};
use backend::policy::trust::evaluate_placement_gate;
use chrono::{Duration, Utc};
use hyper::body;
use jsonwebtoken::{encode, EncodingKey, Header};
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;

#[derive(Clone)]
struct RemediationHarness {
    app: Router,
    token: String,
    operator_id: i32,
    server_id: i32,
    vm_instance_id: i64,
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

    let exp = (Utc::now() + Duration::hours(1)).timestamp();
    let claims = json!({"sub": operator_id, "role": "operator", "exp": exp});
    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(b"integration-secret"),
    )
    .unwrap();

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
        token,
        operator_id,
        server_id,
        vm_instance_id,
    }
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
