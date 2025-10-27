# MCP Host

A Model Context Protocol hosting platform.

## Operator mission-control CLI

Install the mission-control tooling with `pip install -e ./cli` and use the `mcpctl` entry point to drive promotions, governance runs, evaluations, and marketplace inspection without crafting manual HTTP requests. See [cli/README.md](cli/README.md) for full usage documentation and examples.

## Backend configuration

The backend exposes several environment variables to control startup behavior:

| Variable | Description | Default |
| --- | --- | --- |
| `BIND_ADDRESS` | Address the HTTP server listens on. | `0.0.0.0` |
| `BIND_PORT` | Port the HTTP server listens on. | `3000` |
| `ALLOW_MIGRATION_FAILURE` | When set to `true`, allows boot to continue even if database migrations fail. | `false` |
| `K8S_REGISTRY_SECRET_NAME` | Optional Kubernetes secret that is patched after registry credentials refresh. Enables the runtime to roll new Docker auth to pods without manual intervention. | _unset_ |
| `REGISTRY_AUTH_DOCKERCONFIG` | Path to a `dockerconfigjson` file containing registry credentials. Used to seed/refresh the Kubernetes pull secret when auth refresh succeeds. | _unset_ |
| `REGISTRY_AUTH_MAX_AGE_SECONDS` | Maximum age (in seconds) allowed for the `dockerconfigjson` before the health probe reports credentials as expired and recommends rotation. | `86400` |
| `REGISTRY_AUTH_ROTATE_LEAD_SECONDS` | Lead time (in seconds) before the configured max age elapses to trigger proactive credential rotation attempts. | `3600` |
| `REGISTRY_ARCH_TARGETS` | Comma-separated list of target platforms to build and publish (e.g., `linux/amd64,linux/arm64`). | `linux/amd64` |
| `REGISTRY_BUILD_PARALLELISM` | Maximum number of per-architecture builds to execute concurrently. Defaults to the host CPU parallelism capped at the number of targets. | _auto_ |
| `REGISTRY_BUILD_DISABLE_CACHE` | When set to a truthy value, disables Docker layer cache reuse during builds. | `false` |
| `REGISTRY_BUILD_CACHE_FROM` | Comma-separated list of cache image references passed to Docker BuildKit via `cache-from` to seed layer reuse. | _unset_ |

Set these variables in your deployment environment (or a local `.env` file) to adjust how the API service starts.

### Registry push operations

The build service tags and pushes images using the Docker remote API via Bollard. Key behaviors:

* Registry endpoints are emitted via `tracing` with target `registry.push`, including the derived scopes (`repository:<image>:push/pull`) and server ID. Both Docker and Kubernetes runtimes now call the same helper so the emitted telemetry is identical across environments.
* Progress logs include digest discovery lines such as `Manifest published with digest sha256:<hash>` that propagate to the UI. When multi-architecture publishing is enabled, a final manifest digest is emitted after all per-platform pushes complete.
* Authentication failures attempt an automated credential refresh (when a refresher is configured), emitting `auth_refresh_started`, `auth_refresh_succeeded`, or `auth_refresh_failed` metrics and annotating the follow-up `push_retry` event with `reason="auth_refresh"` and the triggering error. Successful refreshes trigger a Kubernetes pull-secret patch when `K8S_REGISTRY_SECRET_NAME` and `REGISTRY_AUTH_DOCKERCONFIG` are set.
* Credential health is polled before each push, emitting `auth_health_reported` (with status, expiry timestamps, and rotation intent) followed by `auth_rotation_started`/`auth_rotation_succeeded`/`auth_rotation_failed` or `auth_rotation_skipped` when the probe recommends proactive rotation. These events carry `rotation_recommended`, `rotation_configured`, and `seconds_until_expiry` fields for alerting pipelines.
* Each registry metric now includes a `platform` field so dashboards can break down success/failure rates per architecture. The manifest publish step emits a dedicated `manifest_published` event describing the aggregated architectures and resulting digest.
* Transient transport errors (I/O, hyper, HTTP client, or timeouts) retry up to `REGISTRY_PUSH_RETRIES` attempts (default `3`) with a short backoff. Override the limit via an environment variable when tuning resilience.
* Usage metrics capture each stage: `tag_started`/`tag_succeeded` for Docker tagging and `push_failed` entries with `attempt=0` for pre-stream failures, giving observability platforms enough context to differentiate tagging issues from push retries.
* Telemetry payloads now include `attempt`, `retry_limit`, `registry_endpoint`, `error_kind`, and `auth_expired` keys so downstream dashboards can surface retry pressure and credential expiry without additional joins. Contract tests cover both REST and SSE payloads to guard this schema.

#### Runbook

1. **Verify telemetry** – search your log aggregator for `target="registry.push" registry push failed` events to identify the failing repository and scope.
2. **Check digest messages** – if `Manifest published` entries are missing, confirm the registry user has push permissions for the derived scopes.
3. **Handle auth expiry** – when logs or metrics show unhealthy credentials (see `auth_health_reported` status or `auth_rotation_failed`), confirm the automated refresh/rotation succeeded (look for `auth_refresh_succeeded` or `auth_rotation_succeeded`). If they fail, rotate credentials and trigger a redeploy; the build log and metrics capture the failure reason for triage.
4. **Transient faults** – for recurring network hiccups, increase `REGISTRY_PUSH_RETRIES` temporarily and monitor retry success events (`registry push succeeded after retry`).
5. **Status recovery** – failed pushes mark the server `error`; once the issue is resolved trigger a redeploy to rebuild and push a fresh image.

### Multi-architecture publishing

The build service orchestrates per-architecture builds using Docker BuildKit and then publishes a manifest list that references each platform-specific image. Configure `REGISTRY_ARCH_TARGETS` to enumerate the platforms to build (for example, `linux/amd64,linux/arm64`). Ensure the host has the necessary emulation shims (e.g., QEMU binfmt) to build non-native architectures and that Docker Buildx is configured for the target platforms.

Build throughput now scales with platform count: `REGISTRY_BUILD_PARALLELISM` (defaulting to the available CPU concurrency) controls how many Docker builds execute at once, and `REGISTRY_BUILD_CACHE_FROM`/`REGISTRY_BUILD_DISABLE_CACHE` provide explicit cache reuse controls. Cached layers speed up iterative builds across architectures, while disabling the cache forces clean rebuilds when debugging.

Manifest publishing requires registry credentials in the configured `dockerconfigjson`. The helper automatically derives the appropriate authorization header and pushes the manifest via the registry HTTP API, emitting a `manifest_published` metric with the participating architectures and resulting digest. After a successful publish, the service prunes manifests for architectures no longer produced, emitting `manifest_prune_started`/`manifest_prune_succeeded`/`manifest_prune_failed` metrics so operators can verify registry hygiene.

## Artifact persistence

Build metadata is now persisted in Postgres so other subsystems (marketplace, runtime policy, evaluations) can reason about concrete artifacts instead of ephemeral logs. Successful git-driven builds insert a record into `build_artifact_runs` capturing:

* `server_id`, source repository/branch/revision, and the resolved `local_image`/`registry_image` pairing.
* Timestamps for build start/completion, the manifest tag/digest (when available), and whether the run produced a multi-architecture manifest.
* Credential lifecycle outcomes (`auth_refresh_*`, `auth_rotation_*`, `credential_health_status`) plus a stable `status` column (`succeeded` today, reserved for richer states later).

Each referenced platform is normalized into `build_artifact_platforms` with the pushed image reference, digest, and per-platform credential/refresh booleans. These rows power queries for architecture coverage, digest-to-run lookups, and future marketplace automation that needs to correlate registry entries back to build outcomes. The persistence layer lives in `backend/src/artifacts.rs` and is guarded by a machine-readable comment so downstream tooling can discover the schema contract automatically.

### Marketplace catalog API

Operators can now query `/api/marketplace` to browse the persisted artifact ledger. The backend hydrates responses directly from `build_artifact_runs`/`build_artifact_platforms`, joining server metadata so every entry advertises:

* Build provenance (source repo/branch/revision), manifest tag/digest, and the registry image reference that was published.
* Architecture coverage plus per-platform credential health so policy engines can gate promotions on multi-arch readiness.
* Derived health tiers (e.g., `gold:Router`, `watchlist:Slack`) computed from build status and credential posture, enabling tier-based filters in the UI or CLI.

Supported query parameters:

| Parameter | Description |
| --- | --- |
| `server_type` | Filter by server type (e.g., `Router`, `PostgreSQL`). |
| `status` | Filter by build run status (`succeeded`, `failed`, etc.). |
| `tier` | Filter by derived tier string. Tiers are case-insensitive. |
| `q` | Performs a fuzzy `ILIKE` search across server name, manifest tag/digest, and image references. |
| `limit` | Caps the number of returned rows (defaults to `50`, maximum `200`). |

The response schema is described in `backend/src/marketplace.rs` (guarded by a `key: marketplace-catalog` comment for downstream automation). Each artifact also includes a `health` block listing any issues so lifecycle tooling can stage promotions or credential rotations with awareness of current risk.

### Runtime policy engine & orchestrator

`backend/src/policy.rs` now models an authoritative placement brain that every runtime invocation flows through. The engine hydrates marketplace metadata before a launch, deriving tier and health classifications from the latest `build_artifact_runs`/`build_artifact_platforms` entries and reconciling them with the requested configuration. Decisions capture:

* The chosen backend (`backend`, `candidate_backend`) alongside override notes, allowing us to distinguish between the original request and the enforced executor.
* The image reference that will be booted, reusing promoted registry artifacts when available and falling back to default catalog images when no ledger entry exists.
* Whether a git build is required, any capability requirements (`gpu`, `image-build`, etc.), and evaluation gating derived from artifact health to enforce staging and certification flows.

### Evaluation certifications & gating

Evaluations are now bound to immutable artifacts via the `evaluation_certifications` table (migration `0023_create_evaluation_certifications.sql`). Each row ties a `build_artifact_run` and `manifest_digest` to a target `tier`, `policy_requirement`, certification `status` (`pending`, `pass`, or `fail`), optional evidence payload, and validity window (`valid_from`/`valid_until`). The API exposes new operator endpoints:

* `GET /api/artifacts/:id/evaluations` lists certifications for a build run.
* `POST /api/artifacts/:id/evaluations` upserts a certification record; when the payload omits a digest the backend falls back to the persisted run digest.
* `POST /api/evaluations/:id/retry` resets a certification to `pending`, clearing the validity window so another evaluation can execute.

`RuntimePolicyEngine::evaluate` now loads the latest certification per policy requirement for the selected tier and enforces gating: deployments proceed only when every requirement has an active `pass`. Missing, expired, pending, or failed certifications emit `evaluation:*` notes and flip `evaluation_required` to `true`, blocking launches until operators submit fresh evidence. Healthy, certified artifacts record `evaluation:certified:<tier>:<requirement>` notes for audit visibility. Regression coverage lives in the `policy::tests` module (`backend/src/policy.rs`) to ensure the policy engine refuses to launch uncertified digests.

The runtime policy schema was expanded in migration `backend/migrations/0022_enhance_runtime_policy_decisions.sql` to persist executor metadata (`candidate_backend`, `capability_requirements`, `capabilities_satisfied`, `executor_name`) in addition to the existing manifest digest, artifact run ID, tier, and policy notes. This audit log powers lifecycle automation, rollback decisions, and credential rotations.

Policy outcomes are now enforced through `backend/src/runtime.rs`, which introduces a `RuntimeOrchestrator`. The orchestrator registers pluggable executors (Docker, Kubernetes today, VM-friendly traits tomorrow) and dispatches launches/stop/delete/log operations based on the recorded policy decision. Each executor advertises capabilities through a `RuntimeExecutorDescriptor`, letting the policy engine satisfy GPU or build requirements before launch. Backend assignments are cached per server to streamline follow-up actions such as log streaming.

The `RuntimePolicyEngine` continues to be exposed to the web API via an Axum `Extension`, enabling CLI or control-surface features to reuse the same policy vocabulary without duplicating logic.

Operators can now fetch attestation posture directly via `GET /api/servers/:id/vm`, which returns the latest VM instances, attestation statuses, isolation tiers, and lifecycle events recorded in `runtime_vm_instances`/`runtime_vm_events`. The `mcpctl policy vm` command consumes this endpoint to render active instance summaries and highlight fallback conditions alongside timestamps so on-call engineers no longer need to query the database by hand when validating confidential workload launches.

### Lifecycle governance workflows

Promotion, rollback, and credential-rotation workflows now have a persistent home backed by migration `backend/migrations/0024_create_governance_workflows.sql`. Operators can define named governance workflows per tier, hydrate them with ordered steps, and initiate workflow runs that target a specific manifest digest or build artifact. The runtime policy engine consults these runs during placement: when no completed promotion exists for the requested tier/digest pair, the decision includes `governance:missing-promotion:*` notes and marks `governance_required = true`, causing the `RuntimeOrchestrator` to pause the launch and set the server status to `pending-governance`.

The governance service lives in `backend/src/governance/` and exposes a typed engine (`GovernanceEngine`) plus REST/SSE routes:

* `GET /api/governance/workflows` / `POST /api/governance/workflows` – list and create reusable workflow definitions.
* `POST /api/governance/workflows/:id/runs` – start a run for an artifact/tier, automatically seeding step run rows.
* `GET /api/governance/runs/:id` – fetch run status, including per-step state and audit log entries.
* `POST /api/governance/runs/:id/status` – transition a run, append audit notes, and, on completion, enqueue a runtime redeploy for the associated policy decision.
* `GET /api/governance/runs/:id/stream` – emit a single-event SSE snapshot of the run for lightweight operator dashboards.

Route handlers publish updates to the existing job queue so completed promotion runs automatically retrigger deployments. The governance engine also links completed runs back to `runtime_policy_decisions`, ensuring audit trails capture which workflow certified a placement. Contract tags (`key: governance-workflows`, `key: governance-api`) guard the engine and HTTP module for downstream automation.

### Release-train promotions

Release trains now have first-class schema support (`backend/migrations/0025_create_promotion_tracks.sql`) tying marketplace digests to promotion tracks and gated runtime placement:

* `promotion_tracks` define ordered stage names, owning users, and the governance workflow that should execute each promotion.
* `artifact_promotions` capture every scheduled promotion for a manifest digest, recording status transitions (`scheduled`, `in_progress`, `approved`, `active`, `rolled_back`) and the linked governance workflow run.

The marketplace API (`backend/src/marketplace.rs`) now surfaces promotion lineage for every artifact. Responses include the active promotion snapshot plus a stage-by-stage history so operators can trace which track and stage delivered the currently running artifact.

New REST endpoints under `/api/promotions` (implemented in `backend/src/promotions.rs`) expose release-train controls:

* `GET /api/promotions/tracks` – list promotion tracks owned by the caller, including configured stages and linked workflows.
* `POST /api/promotions/schedule` – schedule a promotion for a manifest digest/stage. The handler validates stage ordering, records the promotion, and automatically launches the configured governance workflow.
* `POST /api/promotions/:id/approve` – capture manual checkpoint approvals before governance completes.
* `GET /api/promotions/history` – filter promotion records by track or digest for operator dashboards.

The runtime policy engine consumes promotion records before each placement. Only digests with an **active** promotion for the requested tier bypass governance holds; any other state adds `promotion:*` notes to the persisted decision and blocks deployment until the release train advances. Governance workflow status updates synchronize back to `artifact_promotions`, marking successful runs active and rolling back failed attempts, so the runtime always enforces the freshest promotion truth.

### Telemetry consumer audit

The enriched registry telemetry is ingested by several non-UI paths:

| Consumer | Location | Handling | Notes |
| --- | --- | --- | --- |
| Usage metrics table | `backend/migrations/0001_create_tables.sql` | `details` column is `JSONB`, so new `tag_*` and `push_*` fields are stored without schema changes. | Verified that registry-specific keys persist end-to-end. |
| Metrics REST API | `backend/src/servers.rs#get_metrics` | Returns the raw `details` payload for each event. | Snapshot test ensures registry payloads retain keys like `attempt`, `retry_limit`, and `auth_expired`. |
| Metrics SSE stream | `backend/src/servers.rs#stream_metrics` | Serializes each `Metric` with the full `details` object. | Contract test asserts the SSE JSON contains `attempt`, `retry_limit`, `registry_endpoint`, and retry reasons. |

No separate analytics jobs or alert rules exist yet; future consumers should rely on the documented payload contract above.
