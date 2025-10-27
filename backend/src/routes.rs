use axum::{
    routing::{delete, get, patch, post},
    Router,
};

use crate::{
    auth, capabilities, domains, evaluation, file_store, governance, ingestion, intelligence,
    invocations, marketplace, organizations, promotions, secrets, servers, services, vector_dbs,
    workflows,
};

pub fn api_routes() -> Router {
    Router::new()
        .route("/api/marketplace", get(marketplace::list_marketplace))
        .route("/api/register", post(auth::register_user))
        .route("/api/login", post(auth::login_user))
        .route("/api/logout", post(auth::logout_user))
        .route("/api/me", get(auth::current_user))
        .route(
            "/api/servers",
            get(servers::list_servers).post(servers::create_server),
        )
        .route("/api/servers/:id/start", post(servers::start_server))
        .route("/api/servers/:id/stop", post(servers::stop_server))
        .route("/api/servers/:id/redeploy", post(servers::redeploy_server))
        .route("/api/servers/:id/webhook", post(servers::webhook_redeploy))
        .route("/api/servers/:id/github", post(servers::github_webhook))
        .route("/api/servers/:id/invoke", post(servers::invoke_server))
        .route("/api/servers/:id/manifest", get(servers::get_manifest))
        .route(
            "/api/servers/:id/client-config",
            get(servers::client_config),
        )
        .route(
            "/api/servers/:id/capabilities",
            get(capabilities::list_capabilities),
        )
        .route("/api/servers/:id", delete(servers::delete_server))
        .route("/api/servers/:id/logs", get(servers::server_logs))
        .route("/api/servers/:id/logs/history", get(servers::stored_logs))
        .route("/api/servers/:id/logs/stream", get(servers::stream_logs))
        .route(
            "/api/servers/:id/metrics",
            get(servers::get_metrics).post(servers::post_metric),
        )
        .route(
            "/api/servers/:id/metrics/stream",
            get(servers::stream_metrics),
        )
        .route("/api/servers/stream", get(servers::stream_status))
        .route(
            "/api/servers/:id/services",
            get(services::list_services).post(services::create_service),
        )
        .route(
            "/api/servers/:id/services/:service_id",
            patch(services::update_service).delete(services::delete_service),
        )
        .route(
            "/api/servers/:id/secrets",
            get(secrets::list_secrets).post(secrets::create_secret),
        )
        .route(
            "/api/servers/:id/secrets/:secret_id",
            get(secrets::get_secret)
                .patch(secrets::update_secret)
                .delete(secrets::delete_secret),
        )
        .route(
            "/api/servers/:id/domains",
            get(domains::list_domains).post(domains::create_domain),
        )
        .route(
            "/api/servers/:id/domains/:domain_id",
            delete(domains::delete_domain),
        )
        .route(
            "/api/servers/:id/files",
            get(file_store::list_files).post(file_store::upload_file),
        )
        .route(
            "/api/servers/:id/files/:file_id",
            get(file_store::download_file).delete(file_store::delete_file),
        )
        .route(
            "/api/vector-dbs",
            get(vector_dbs::list_vector_dbs).post(vector_dbs::create_vector_db),
        )
        .route("/api/vector-dbs/:id", delete(vector_dbs::delete_vector_db))
        .route(
            "/api/ingestion-jobs",
            get(ingestion::list_jobs).post(ingestion::create_job),
        )
        .route("/api/ingestion-jobs/:id", delete(ingestion::delete_job))
        .route(
            "/api/servers/:id/invocations",
            get(invocations::list_invocations),
        )
        .route(
            "/api/servers/:id/eval/tests",
            get(evaluation::list_tests).post(evaluation::create_test),
        )
        .route("/api/servers/:id/eval/run", post(evaluation::run_tests))
        .route(
            "/api/servers/:id/eval/results",
            get(evaluation::list_results),
        )
        .route(
            "/api/intelligence/servers/:id/scores",
            get(intelligence::list_scores),
        )
        .route(
            "/api/artifacts/:id/evaluations",
            get(evaluation::list_certifications).post(evaluation::submit_certification),
        )
        .route("/api/evaluations", get(evaluation::list_all_results))
        .route(
            "/api/evaluations/:id/retry",
            post(evaluation::retry_certification),
        )
        .route("/api/evaluations/summary", get(evaluation::scores_summary))
        .merge(governance::routes())
        .merge(promotions::routes())
        .merge(workflows::routes())
        .merge(organizations::routes())
}
