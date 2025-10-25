# MCP Host

A Model Context Protocol hosting platform.

## Backend configuration

The backend exposes several environment variables to control startup behavior:

| Variable | Description | Default |
| --- | --- | --- |
| `BIND_ADDRESS` | Address the HTTP server listens on. | `0.0.0.0` |
| `BIND_PORT` | Port the HTTP server listens on. | `3000` |
| `ALLOW_MIGRATION_FAILURE` | When set to `true`, allows boot to continue even if database migrations fail. | `false` |
| `K8S_REGISTRY_SECRET_NAME` | Optional Kubernetes secret that is patched after registry credentials refresh. Enables the runtime to roll new Docker auth to pods without manual intervention. | _unset_ |
| `REGISTRY_AUTH_DOCKERCONFIG` | Path to a `dockerconfigjson` file containing registry credentials. Used to seed/refresh the Kubernetes pull secret when auth refresh succeeds. | _unset_ |
| `REGISTRY_ARCH_TARGETS` | Comma-separated list of target platforms to build and publish (e.g., `linux/amd64,linux/arm64`). | `linux/amd64` |

Set these variables in your deployment environment (or a local `.env` file) to adjust how the API service starts.

### Registry push operations

The build service tags and pushes images using the Docker remote API via Bollard. Key behaviors:

* Registry endpoints are emitted via `tracing` with target `registry.push`, including the derived scopes (`repository:<image>:push/pull`) and server ID. Both Docker and Kubernetes runtimes now call the same helper so the emitted telemetry is identical across environments.
* Progress logs include digest discovery lines such as `Manifest published with digest sha256:<hash>` that propagate to the UI. When multi-architecture publishing is enabled, a final manifest digest is emitted after all per-platform pushes complete.
* Authentication failures attempt an automated credential refresh (when a refresher is configured), emitting `auth_refresh_started`, `auth_refresh_succeeded`, or `auth_refresh_failed` metrics and annotating the follow-up `push_retry` event with `reason="auth_refresh"` and the triggering error. Successful refreshes trigger a Kubernetes pull-secret patch when `K8S_REGISTRY_SECRET_NAME` and `REGISTRY_AUTH_DOCKERCONFIG` are set.
* Each registry metric now includes a `platform` field so dashboards can break down success/failure rates per architecture. The manifest publish step emits a dedicated `manifest_published` event describing the aggregated architectures and resulting digest.
* Transient transport errors (I/O, hyper, HTTP client, or timeouts) retry up to `REGISTRY_PUSH_RETRIES` attempts (default `3`) with a short backoff. Override the limit via an environment variable when tuning resilience.
* Usage metrics capture each stage: `tag_started`/`tag_succeeded` for Docker tagging and `push_failed` entries with `attempt=0` for pre-stream failures, giving observability platforms enough context to differentiate tagging issues from push retries.
* Telemetry payloads now include `attempt`, `retry_limit`, `registry_endpoint`, `error_kind`, and `auth_expired` keys so downstream dashboards can surface retry pressure and credential expiry without additional joins. Contract tests cover both REST and SSE payloads to guard this schema.

#### Runbook

1. **Verify telemetry** – search your log aggregator for `target="registry.push" registry push failed` events to identify the failing repository and scope.
2. **Check digest messages** – if `Manifest published` entries are missing, confirm the registry user has push permissions for the derived scopes.
3. **Handle auth expiry** – when logs show `authentication required`, confirm the automated refresh succeeded (look for `auth_refresh_succeeded`). If it failed, rotate credentials and trigger a redeploy; the build log and metrics (`auth_refresh_failed`) include the refresh error for triage.
4. **Transient faults** – for recurring network hiccups, increase `REGISTRY_PUSH_RETRIES` temporarily and monitor retry success events (`registry push succeeded after retry`).
5. **Status recovery** – failed pushes mark the server `error`; once the issue is resolved trigger a redeploy to rebuild and push a fresh image.

### Multi-architecture publishing

The build service orchestrates per-architecture builds using Docker BuildKit and then publishes a manifest list that references each platform-specific image. Configure `REGISTRY_ARCH_TARGETS` to enumerate the platforms to build (for example, `linux/amd64,linux/arm64`). Ensure the host has the necessary emulation shims (e.g., QEMU binfmt) to build non-native architectures and that Docker Buildx is configured for the target platforms.

Manifest publishing requires registry credentials in the configured `dockerconfigjson`. The helper automatically derives the appropriate authorization header and pushes the manifest via the registry HTTP API, emitting a `manifest_published` metric with the participating architectures and resulting digest.

### Telemetry consumer audit

The enriched registry telemetry is ingested by several non-UI paths:

| Consumer | Location | Handling | Notes |
| --- | --- | --- | --- |
| Usage metrics table | `backend/migrations/0001_create_tables.sql` | `details` column is `JSONB`, so new `tag_*` and `push_*` fields are stored without schema changes. | Verified that registry-specific keys persist end-to-end. |
| Metrics REST API | `backend/src/servers.rs#get_metrics` | Returns the raw `details` payload for each event. | Snapshot test ensures registry payloads retain keys like `attempt`, `retry_limit`, and `auth_expired`. |
| Metrics SSE stream | `backend/src/servers.rs#stream_metrics` | Serializes each `Metric` with the full `details` object. | Contract test asserts the SSE JSON contains `attempt`, `retry_limit`, `registry_endpoint`, and retry reasons. |

No separate analytics jobs or alert rules exist yet; future consumers should rely on the documented payload contract above.
