use axum::{routing::post, Extension, Router};
use backend::db::runtime_vm_remediation_runs::{
    ensure_remediation_run, EnsureRemediationRunRequest,
};
use backend::policy::trust::evaluate_placement_gate;
use backend::trust::TrustRegistryView;
use chrono::{Duration, Utc};
use hyper::{Body, Request, StatusCode};
use jsonwebtoken::{encode, EncodingKey, Header};
use serde_json::json;
use sqlx::PgPool;
use tower::ServiceExt;

#[sqlx::test]
#[ignore = "requires DATABASE_URL with Postgres server"]
async fn transition_endpoint_records_event(pool: PgPool) {
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    let user_id: i32 =
        sqlx::query_scalar("INSERT INTO users (email, password_hash) VALUES ($1, $2) RETURNING id")
            .bind("operator@example.com")
            .bind("hashed")
            .fetch_one(&pool)
            .await
            .unwrap();

    let server_id: i32 = sqlx::query_scalar(
        "INSERT INTO mcp_servers (owner_id, name, server_type, config, status, api_key) VALUES ($1, $2, $3, '{}'::jsonb, $4, $5) RETURNING id",
    )
    .bind(user_id)
    .bind("edge-node")
    .bind("virtual-machine")
    .bind("active")
    .bind("test-key")
    .fetch_one(&pool)
    .await
    .unwrap();

    let vm_instance_id: i64 = sqlx::query_scalar(
        "INSERT INTO runtime_vm_instances (server_id, instance_id) VALUES ($1, $2) RETURNING id",
    )
    .bind(server_id)
    .bind("vm-test-1")
    .fetch_one(&pool)
    .await
    .unwrap();

    std::env::set_var("JWT_SECRET", "integration-secret");
    let exp = (Utc::now() + Duration::hours(1)).timestamp();
    let claims = json!({"sub": user_id, "role": "operator", "exp": exp});
    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(b"integration-secret"),
    )
    .unwrap();

    let app = Router::new()
        .route(
            "/api/trust/registry/:instance_id/transition",
            post(backend::trust::transition_registry_state),
        )
        .layer(Extension(pool.clone()));

    let body = json!({
        "attestation_status": "trusted",
        "lifecycle_state": "restored",
        "remediation_state": null,
        "remediation_attempts": 0,
        "freshness_deadline": null,
        "provenance_ref": null,
        "provenance": null,
        "transition_reason": "integration-test",
        "metadata": {"source": "test"},
        "expected_version": null
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/trust/registry/{}/transition", vm_instance_id))
                .header("Authorization", format!("Bearer {}", token))
                .header("Content-Type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
    let view: TrustRegistryView = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(view.attestation_status, "trusted");
    assert_eq!(view.lifecycle_state, "restored");
    assert_eq!(view.remediation_attempts, 0);

    let history_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM runtime_vm_trust_history WHERE runtime_vm_instance_id = $1",
    )
    .bind(vm_instance_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(history_count, 1);
}

#[sqlx::test]
#[ignore = "requires DATABASE_URL with Postgres server"]
async fn placement_gate_blocks_active_remediation(pool: PgPool) {
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    let user_id: i32 =
        sqlx::query_scalar("INSERT INTO users (email, password_hash) VALUES ($1, $2) RETURNING id")
            .bind("gatekeeper@example.com")
            .bind("hashed")
            .fetch_one(&pool)
            .await
            .unwrap();

    let server_id: i32 = sqlx::query_scalar(
        "INSERT INTO mcp_servers (owner_id, name, server_type, config, status, api_key) VALUES ($1, $2, $3, '{}'::jsonb, $4, $5) RETURNING id",
    )
    .bind(user_id)
    .bind("gate-test")
    .bind("virtual-machine")
    .bind("active")
    .bind("gate-key")
    .fetch_one(&pool)
    .await
    .unwrap();

    let vm_instance_id: i64 = sqlx::query_scalar(
        "INSERT INTO runtime_vm_instances (server_id, instance_id) VALUES ($1, $2) RETURNING id",
    )
    .bind(server_id)
    .bind("vm-gate-1")
    .fetch_one(&pool)
    .await
    .unwrap();

    let run = ensure_remediation_run(
        &pool,
        EnsureRemediationRunRequest {
            runtime_vm_instance_id: vm_instance_id,
            playbook_key: "default-vm-remediation",
            playbook_id: None,
            metadata: None,
            automation_payload: None,
            approval_required: true,
            assigned_owner_id: None,
            sla_duration_seconds: None,
        },
    )
    .await
    .unwrap()
    .expect("run");

    let gate = evaluate_placement_gate(&pool, server_id)
        .await
        .unwrap()
        .expect("gate");

    assert!(
        gate.blocked,
        "expected placement gate to block active remediation"
    );
    assert!(gate
        .notes
        .iter()
        .any(|note| note.contains("policy_hook:remediation_gate=active-run")));
    assert!(gate
        .notes
        .iter()
        .any(|note| note.contains(&format!("remediation:awaiting-approval:{}", run.id))));
}

#[sqlx::test]
#[ignore = "requires DATABASE_URL with Postgres server"]
async fn placement_gate_flags_structural_failure(pool: PgPool) {
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    let user_id: i32 =
        sqlx::query_scalar("INSERT INTO users (email, password_hash) VALUES ($1, $2) RETURNING id")
            .bind("structural@example.com")
            .bind("hashed")
            .fetch_one(&pool)
            .await
            .unwrap();

    let server_id: i32 = sqlx::query_scalar(
        "INSERT INTO mcp_servers (owner_id, name, server_type, config, status, api_key) VALUES ($1, $2, $3, '{}'::jsonb, $4, $5) RETURNING id",
    )
    .bind(user_id)
    .bind("failure-test")
    .bind("virtual-machine")
    .bind("active")
    .bind("fail-key")
    .fetch_one(&pool)
    .await
    .unwrap();

    let vm_instance_id: i64 = sqlx::query_scalar(
        "INSERT INTO runtime_vm_instances (server_id, instance_id) VALUES ($1, $2) RETURNING id",
    )
    .bind(server_id)
    .bind("vm-failure-1")
    .fetch_one(&pool)
    .await
    .unwrap();

    let run = ensure_remediation_run(
        &pool,
        EnsureRemediationRunRequest {
            runtime_vm_instance_id: vm_instance_id,
            playbook_key: "default-vm-remediation",
            playbook_id: None,
            metadata: None,
            automation_payload: None,
            approval_required: false,
            assigned_owner_id: None,
            sla_duration_seconds: None,
        },
    )
    .await
    .unwrap()
    .expect("run");

    sqlx::query(
        "UPDATE runtime_vm_remediation_runs SET status = 'failed', failure_reason = $2, completed_at = NOW(), approval_state = 'approved' WHERE id = $1",
    )
    .bind(run.id)
    .bind("policy-denied")
    .execute(&pool)
    .await
    .unwrap();

    let gate = evaluate_placement_gate(&pool, server_id)
        .await
        .unwrap()
        .expect("gate");

    assert!(
        gate.blocked,
        "expected placement gate to block after structural failure"
    );
    assert!(
        gate.notes
            .iter()
            .any(|note| note
                .contains("policy_hook:remediation_gate=failure:policy-denied:structural"))
    );
}
