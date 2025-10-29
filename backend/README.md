# Backend Runtime Notes

## Libvirt Executor Configuration
The runtime can now launch confidential VMs directly against libvirt. The executor reads its configuration from environment variables so deployments can switch drivers without rebuilding the binary. Key variables include:

| Variable | Description |
| --- | --- |
| `VM_PROVISIONER_DRIVER` | Set to `libvirt` to enable the libvirt executor, or leave as `http` to use the legacy hypervisor API. |
| `LIBVIRT_CONNECTION_URI` | Libvirt connection URI (for example `qemu:///system` or `qemu+ssh://host/system`). |
| `LIBVIRT_USERNAME` / `LIBVIRT_PASSWORD` | Optional credentials used during connection negotiation. Passwords can also be supplied via `LIBVIRT_PASSWORD_FILE`. |
| `LIBVIRT_DEFAULT_ISOLATION_TIER` | Default isolation tier applied when policy decisions omit an explicit value. |
| `LIBVIRT_DEFAULT_MEMORY_MIB` / `LIBVIRT_DEFAULT_VCPU_COUNT` | Baseline VM sizing in MiB and vCPU count. |
| `LIBVIRT_LOG_TAIL` | Number of console lines fetched per request; affects the HTTP API and CLI log views. |
| `LIBVIRT_NETWORK_TEMPLATE` | JSON template merged into the generated domain XML network stanza. |
| `LIBVIRT_VOLUME_TEMPLATE` | JSON template describing the boot volume path, driver, and target bus. |
| `LIBVIRT_GPU_POLICY` | JSON policy describing GPU passthrough requirements (devices, enablement flag). |
| `LIBVIRT_CONSOLE_SOURCE` | Optional console source (`pty`, `tcp`, etc.) inserted into the domain XML. |

All JSON templates are validated during startup. Invalid values will cause the process to abort with an explanatory panic so operators can fix misconfigurations early.

## Deployment Prerequisites
Deployments must install the libvirt daemon and ensure the runtime has permission to create and control domains. On Linux hosts this typically requires adding the service account to the `libvirt` group and enabling `libvirtd` + `virtlogd`. When running over SSH (`qemu+ssh://`) make sure host keys are trusted and the runtime user can read any required private keys.

Enabling the libvirt executor also requires building the backend with the `libvirt-executor` cargo feature so the native driver is compiled in.

## Secret Rotation
Passwords can be rotated without restarts by updating `LIBVIRT_PASSWORD_FILE` to point at the new secret and sending a SIGHUP (or restarting the process). The runtime records only sanitized snapshots of credentials in `runtime_vm_instances`, ensuring API consumers see whether secrets were supplied without exposing raw values.

## Troubleshooting Console Access
If log retrieval returns empty output, verify `LIBVIRT_CONSOLE_SOURCE` matches the domain's serial device configuration and that `LIBVIRT_LOG_TAIL` is large enough to capture recent lines. The new streaming test covers channel setup end-to-end; if streaming fails in production, confirm `virtlogd` is running and that SELinux/AppArmor policies allow read access to the console device.

## Attestation Trust Fabric

The VM runtime now persists posture transitions in a dedicated `runtime_vm_trust_history` table and mirrors the latest lifecycle snapshot in `runtime_vm_trust_registry`. Each attestation outcome recorded through `policy::trust::persist_vm_attestation_outcome` emits a `pg_notify` event, advances the lifecycle state machine (`suspect → quarantined → remediating → restored`), stamps remediation attempt counters, and records attestation provenance. Registry writes use optimistic locking so scheduler/policy/operator actions can coordinate without clobbering state.

Key integration points:

- **Scheduler:** `evaluations::scheduler` now promotes distrusted instances into the `remediating` lifecycle state while cancelling refresh jobs, preserving fallback timestamps, and keeping optimistic registry versions in sync. Trusted states resume the cadence automatically when restored. The refresh loop also reuses the placement gate (see below) so certification jobs respect `policy_hook:remediation_gate=*` vetoes before dispatching new work.
- **Trust notifications:** `trust::spawn_trust_listener` subscribes to the `runtime_vm_trust_transition` channel, recalculates evaluation plans in real time, forwards lifecycle/provenance metadata to SSE clients, and triggers intelligence recomputes whenever posture changes arrive from Postgres.
- **Policy engine:** placement decisions hydrate the latest trust event via `runtime_vm_trust_history`, emit `vm:trust-*` notes for lifecycle, remediation counts, and provenance, and rely on registry snapshots when determining backend overrides. The remediation-aware placement gate annotates notes such as `policy_hook:remediation_gate=active-run:*` and classifies failures as `structural`, `transient`, or `cancelled` so downstream schedulers and operators have unambiguous veto reasons.
- **Runtime orchestrator:** placement launches now consult `runtime_vm_trust_registry` and block deployments when lifecycles are `quarantined` or remediation windows are stale, flipping servers into `pending-remediation`/`pending-attestation` until evidence is refreshed.
- **Operator tooling:** `/api/servers/:id/vm`, `/api/evaluations`, and the CLI now expose lifecycle badges, remediation attempt counts, freshness deadlines, and provenance references so consoles can highlight blocked evidence and remediation activity without manual SQL queries.
- **Intelligence scoring:** scoring logic folds lifecycle state, remediation attempts, freshness deadlines, and provenance hints into capability notes and evidence payloads. Servers with degraded posture incur score penalties proportional to remediation churn and stale evidence windows.

Refer to `progress.md` for the operational rollout plan covering notification listeners, CLI affordances, and intelligence feedback loops built on top of the trust registry.

### Trust control plane API

The trust registry now exposes a REST and streaming control surface so remediation services, policy engines, and operator tooling can coordinate without direct database access.

- **Registry listing:** `GET /api/trust/registry` returns the latest lifecycle snapshot for the authenticated owner. Query parameters (`server_id`, `lifecycle_state`, `attestation_status`, `stale`) provide filtered views.
- **Instance detail:** `GET /api/trust/registry/:vm_instance_id` returns the most recent posture for a specific VM instance, while `GET /api/trust/registry/:vm_instance_id/history` surfaces lifecycle transitions capped by a configurable limit.
- **State transitions:** `POST /api/trust/registry/:vm_instance_id/transition` applies guarded state changes with optimistic concurrency tokens. The handler persists a new history row and rebroadcasts the enriched payload to downstream consumers.
- **Streaming events:** `GET /api/trust/registry/stream` streams SSE payloads that mirror the Postgres NOTIFY channel. Filters match the REST list parameters so dashboards and the CLI can watch targeted lifecycles without custom fan-out code.

The remediation orchestrator listens to the in-process broadcast channel, starting automation playbooks when quarantined lifecycles appear. Placeholder automation marks runs complete after basic verification; replace the stub in `backend/src/remediation.rs` as production playbooks mature. Use migration `0033_remediation_orchestrator.sql` before deploying the control plane.

## Remediation Control Plane & Execution Engine

Remediation is now a first-class control plane built on top of persistent tables introduced in
`0034_remediation_control_plane.sql` and extended by `0035_remediation_accelerator_posture.sql`:

- `runtime_vm_remediation_playbooks` stores catalog metadata for each playbook (`playbook_key`, executor type, owner assignment, `approval_required`, `sla_duration_seconds`, version column for optimistic locking, and arbitrary JSONB metadata). Updates must supply the prior `version` to avoid stomping concurrent edits. The Axum control plane and CLI surface both use this table to render catalog listings and enforce ownership.
- `runtime_vm_remediation_runs` represents individual automation attempts. New runs start in `status = 'pending'` with `approval_state` seeded to `pending` or `auto-approved` depending on the playbook. Additional columns track assigned owner, SLA deadline, cancellation fields, structured metadata, and a `failure_reason` enum string so policy consumers can distinguish transient vs. structural failures.
- `runtime_vm_remediation_artifacts` captures log bundles, evidence attachments, and automation outputs with JSON metadata so downstream policy engines can ingest remediation intelligence.
- `runtime_vm_accelerator_posture` stores accelerator inventory posture (`accelerator_id`, hardware
  `kind`, `posture` tag, policy feedback signals, and opaque metadata) keyed by VM instance. The
  remediation API ingests this table whenever scenario metadata supplies accelerator fixtures so
  placement policy, telemetry, and SSE payloads share a consistent view of accelerator governance.
  Scenario metadata now publishes structured `policy_gate` information that pairs remediation
  veto hooks with per-accelerator gating reasons to explain why automation remains blocked.

The orchestrator (`backend/src/remediation.rs`) wires these tables into a stateful execution engine:

- `RemediationExecutor` is a pluggable trait with default shell, Ansible, and cloud API adapters. Each executor streams `RemediationLogEvent` structures (stdout/stderr/system), supports cancellation tokens, and returns structured `RemediationExitStatus` values annotated with a `RemediationFailureReason`. The failure taxonomy now differentiates policy denials, playbook bugs, dependency outages, timeouts, executor availability issues, and other transient vs. structural causes so policy consumers can respond accordingly.
- A queue worker (`dispatch_next_run`) dequeues approved `runtime_vm_remediation_runs`, flips the trust registry into `remediation:automation-running`, and launches executors asynchronously. Execution results checkpoint run status, persist logs as artifacts, and write typed remediation states (`automation-complete`, `automation-failed`, `transient-failure`, etc.) back into `runtime_vm_trust_registry` using optimistic locking.
- The quarantine event listener now materializes playbook-backed runs with provenance metadata, owner assignment, and SLA deadlines, distinguishing between approval-gated and automation-ready lifecycles.

Operators should consult the remediation API/CLI roadmap before enabling automated playbooks in production. The data contracts documented above are considered stable for downstream integrations, and telemetry should consume the high-signal log artifacts rather than raw executor stdout.

### Remediation API & CLI Surfaces

- **REST:**
  - `GET/POST /api/trust/remediation/playbooks` for catalog listing and creation with optimistic locking metadata (`remediation_surface: playbook-catalog`).
  - `GET/PATCH/DELETE /api/trust/remediation/playbooks/:id` for retrieval, edits, and cleanup guarded by the `version` token.
  - `GET/POST /api/trust/remediation/runs` to inspect lifecycle state and enqueue automation (400 on unknown playbooks, 409 on active runs).
  - `GET /api/trust/remediation/runs/:id` and `POST /api/trust/remediation/runs/:id/approval` to drive approval workflows and examine run metadata.
  - `GET /api/trust/remediation/runs/:id/artifacts` to fetch structured evidence bundles.
  - `GET /api/trust/remediation/stream` for SSE log/status streaming filtered by `run_id`. Stream
    payloads now include `manifest_tags` (derived from playbook/run metadata), aggregated
    `policy_feedback` hooks, structured `policy_gate` objects, and enriched `accelerators` arrays so
    dashboards can correlate chaos-manifest fingerprints, placement veto notes, accelerator posture,
    and gating reasons in a single feed.
- **CLI (`mcpctl remediation`):** new subcommands mirror the REST surface (`playbooks list`, `runs list|get|enqueue|approve|artifacts`, `watch`) with JSON output toggles and structured table rendering to simplify operator workflows. The `watch` renderer now surfaces policy feedback, remediation gates, and accelerator posture summaries alongside status transitions so operators see governance context without parsing raw JSON.

### Remediation workspace lifecycle

Migration `0036_remediation_workspace_lifecycle.sql` introduces four normalized tables that capture
governed remediation drafts and their validation artifacts:

- `runtime_vm_remediation_workspaces` stores high-level metadata (display name, owner, lifecycle
  state, lineage tags, active revision pointer, optimistic locking version).
- `runtime_vm_remediation_workspace_revisions` persists each plan iteration with schema/policy/
  simulation/promotion gate status, lineage labels, validator timestamps, and structured metadata.
- `runtime_vm_remediation_workspace_sandbox_executions` records sandbox simulations (requested by,
  simulator kind, diff snapshot, gate context, lifecycle timestamps, execution outcome).
- `runtime_vm_remediation_workspace_validation_snapshots` captures schema/policy/promotion snapshots
  with recorded notes so operators have an immutable audit trail across gate evaluations.

The Axum API layers these tables into workspace lifecycle endpoints that parallel the remediation
run gate metadata already exposed via SSE:

- `GET /api/trust/remediation/workspaces` and `GET /api/trust/remediation/workspaces/:id` return
  `WorkspaceEnvelope` structures containing the workspace, revision envelopes, gate summaries,
  sandbox executions, and validation snapshots.
- `POST /api/trust/remediation/workspaces` creates a draft workspace, seeding revision `1` and
  activating optimistic locking tokens for subsequent updates.
- `POST /api/trust/remediation/workspaces/:id/revisions` appends a revision, enforcing the caller's
  expected workspace version to guard against concurrent edits.
- `POST /api/trust/remediation/workspaces/:id/revisions/:revision_id/schema` records schema
  validation output with validator gate context and error vectors.
- `POST /api/trust/remediation/workspaces/:id/revisions/:revision_id/policy` captures policy
  feedback/veto metadata and mirrors gate context into validation snapshots.
- `POST /api/trust/remediation/workspaces/:id/revisions/:revision_id/simulation` persists sandbox
  orchestration results (including diff snapshots) so operators can replay simulations before
  promotion.
- `POST /api/trust/remediation/workspaces/:id/revisions/:revision_id/promotion` applies promotion
  status and appends audit notes; optimistic locking covers both the workspace and targeted
  revision.

CLI parity arrives via `mcpctl remediation workspaces` subcommands for listing, retrieving detailed
gate state, creating drafts, creating revisions (with lineage labels/expected versions), recording
schema/policy feedback, capturing sandbox simulations, diffing the latest sandbox payload, and
issuing promotion status updates. Each command accepts `--json` for raw payload emission so harness
automation and dashboards can consume identical artefacts.

Promotion status handlers now return a `promotion_runs` array that captures the
`runtime_vm_instance_id`, promotion gate context, trust posture, automation payload, and promotion
notes for each staged remediation run. The remediation SSE feed mirrors that information in real
time—`RemediationStreamMessage` includes the refreshed `automation_payload` alongside existing gate
context and accelerator posture metadata—so CLI watchers and consoles surface actionable automation
context the moment a promotion creates or refreshes a run. The CLI promotion renderer prints an
"Automation status" table highlighting gate lanes/stages, trust posture, automation payload summaries,
and promotion notes to close the operator feedback loop.

Integration coverage now includes the `remediation_workspace_lifecycle_end_to_end` SQLx test which
drives draft creation, revision iteration, schema/policy validation snapshots, sandbox simulation,
and promotion gates with explicit optimistic locking assertions. The remediation harness executes
the same database-backed tests and now exercises the CLI stack (`mcpctl remediation workspaces`)
to create revisions, record gate outcomes, and complete promotions under the same optimistic locking
tokens surfaced by the REST handlers.
`remediation_workspace_promotion_multiple_targets` extends this suite by simulating nested promotion
targets, verifying per-target playbook selection, promotion gate context propagation, and the
resulting automation metadata for each runtime VM instance.

### Lifecycle console aggregation surface

To connect remediation lifecycle data with trust posture, intelligence scoring, and marketplace
readiness, the backend now exposes a typed aggregation module in
`backend/src/lifecycle_console/mod.rs`. The new `GET /api/console/lifecycle` route returns a
`LifecycleConsolePage` structure that bundles workspaces, active revisions (including recorded gate
snapshots), recent remediation runs, trust registry posture, intelligence score overviews, and the
latest marketplace readiness status keyed by server. Pagination cursors and optional filters
(`lifecycle_state`, `owner_id`, `workspace_key`) keep the response scoped for large tenants while
`run_limit` bounds per-workspace automation detail.

The aggregator stitches together data from `runtime_vm_remediation_workspaces`,
`runtime_vm_remediation_runs`, `runtime_vm_trust_registry`, `capability_intelligence_scores`, and
`build_artifact_runs` using windowed SQLx queries so UI consumers receive normalized payloads without
duplicating join logic. Dedicated helper functions encapsulate the per-table lookups, keeping the
module testable and ready for SSE streaming expansion. See
`backend/tests/lifecycle_console.rs` for an integration scenario that seeds a workspace, trust state,
intelligence score, and marketplace artifact before exercising the new endpoint.

Promotion verdict propagation is now part of the lifecycle surface: both REST snapshots and SSE
deltas include a `promotion_postures` array for each workspace alongside run metadata. Each slice
carries the persisted `posture_verdict` JSON (allowed flag, veto reasons, blended signal metadata,
and remediation hooks) plus track/stage identifiers. Streaming updates emit
`promotion_posture_deltas`/`removed_promotion_ids` so clients can replay posture narrative changes
without rehydrating from the CLI transcripts.

Lifecycle snapshots also expose a `promotion_runs` array so operators can correlate promotion
verdicts with the remediation automation that a promotion staged or refreshed. The backend reuses
the promotion orchestration helper to hydrate the latest per-workspace automation runs, and SSE
delta envelopes now include `promotion_run_deltas`/`removed_promotion_run_ids` so clients can
diff automation refreshes without dropping cached state.

#### Lifecycle SSE event schema

Lifecycle streaming responses (`GET /api/console/lifecycle/stream`) emit Server-Sent Events with
the JSON envelope described below. Promotion automation parity hinges on the `promotion_runs`
collection and the replay-safe `promotion_run_deltas` payload.

Each remediation run snapshot now publishes automation analytics and artifact metadata alongside
trust and intelligence state. The `recent_runs` array surfaces both coarse and precise duration
signals (`duration_seconds`, `duration_ms`, and an `execution_window` timestamp pair), blended retry
context (`retry_attempt`, `retry_limit`, canonical `retry_count`, and a structured `retry_ledger`),
manual override provenance (`override_reason` plus `manual_override.actor_email/actor_id`), and an
`artifacts` array with hydrated provenance details (manifest digest, lane/stage context, track
metadata, manifest tag, registry image, build status, completion timestamp, and duration). Each run
also captures `artifact_fingerprints`—stable SHA-256 digests derived from manifest and track
attributes—and `promotion_verdict` references that link remediation attempts back to the governing
promotion verdict. Marketplace readiness entries mirror these additions with `manifest_digest`,
`manifest_tag`, `registry_image`, and `build_duration_seconds`. SSE deltas populate
`analytics_changes` and `artifact_changes` vectors so streaming consumers can diff retry counts,
overrides, verdict linkage, and artifact rollups without replaying entire snapshots.

```json
{
  "type": "snapshot",                 // "snapshot", "heartbeat", or "error"
  "emitted_at": "2025-12-09T12:00:00Z",
  "cursor": 42,
  "page": {
    "workspaces": [
      {
        "workspace": { "id": 17, "workspace_key": "workspace-alpha", "lifecycle_state": "active" },
        "promotion_runs": [
          {
            "id": 321,
            "status": "pending",
            "playbook": "verify-automation",
            "automation_payload": { "lane": "prod" },
            "promotion_gate_context": { "lane": "prod", "stage": "production" },
            "metadata": { "notes": ["preflight"] }
          }
        ],
        "promotion_postures": [
          {
            "promotion_id": 77,
            "status": "pending",
            "allowed": false,
            "track_name": "stable",
            "track_tier": "tier-1",
            "stage": "production",
            "updated_at": "2025-12-09T00:00:00Z",
            "remediation_hooks": ["policy_hook:remediation_gate"]
          }
        ],
        "recent_runs": [
          {
            "run": { "id": 44, "status": "succeeded", "playbook": "verify-automation" },
            "trust": { "attestation_status": "trusted", "lifecycle_state": "restored" },
            "intelligence": [],
            "marketplace": { "status": "ready", "last_completed_at": "2025-12-08T10:00:00Z" },
            "duration_seconds": 180,
            "duration_ms": 182000,
            "execution_window": {
              "started_at": "2025-12-08T09:52:00Z",
              "completed_at": "2025-12-08T09:55:00Z"
            },
            "retry_attempt": 2,
            "retry_limit": 5,
            "retry_count": 2,
            "retry_ledger": [
              {"attempt": 1, "status": "failed", "observed_at": "2025-12-08T09:53:00Z"},
              {"attempt": 2, "status": "succeeded", "observed_at": "2025-12-08T09:55:00Z"}
            ],
            "override_reason": "manual approval",
            "manual_override": {
              "reason": "manual approval",
              "actor_email": "operator@example.com"
            },
            "artifacts": [
              {
                "manifest_digest": "sha256:artifact",
                "lane": "prod",
                "stage": "production",
                "track_name": "stable",
                "track_tier": "tier-1",
                "manifest_tag": "v1",
                "registry_image": "registry.example/mcp:v1",
                "duration_seconds": 95
              }
            ],
            "artifact_fingerprints": [
              {
                "manifest_digest": "sha256:artifact",
                "fingerprint": "4c4d5c8a4b341f6a9c5e2d5876a9c1f2"
              }
            ],
            "promotion_verdict": {
              "verdict_id": 91,
              "allowed": false,
              "stage": "production",
              "track_name": "stable",
              "track_tier": "tier-1"
            }
          }
        ]
      }
    ]
  },
  "delta": {
    "workspaces": [
      {
        "workspace_id": 17,
        "promotion_run_deltas": [
          {
            "run_id": 321,
            "status": "succeeded",
            "automation_payload_changes": [
              {
                "field": "promotion_run.automation_payload",
                "previous": "{\"lane\":\"prod\"}",
                "current": "{\"lane\":\"prod\",\"result\":\"ok\"}"
              }
            ],
            "gate_context_changes": [],
            "metadata_changes": []
          }
        ],
        "removed_promotion_run_ids": [222],
        "promotion_posture_deltas": [],
        "removed_promotion_ids": []
      }
    ]
  }
}
```

Heartbeat events omit the `page` and `delta` payload, carrying only the envelope metadata. CLI and
frontend consumers should honour the cursor for resume support and replay the delta arrays in
order, applying `removed_*` identifiers after processing the change lists to preserve cache
integrity.

### Backend crate structure

To avoid the duplicate-type compilation failures that occurred when both the library and binary
targets privately re-declared modules, the binary now consumes modules through the library surface.
`src/main.rs` imports from the `backend` crate (`backend::routes::api_routes`,
`backend::job_queue::start_worker`, `backend::runtime::{...}`) instead of `mod` declarations, and the
library exposes the supporting modules (`auth`, `domains`, `ingestion`, `routes`, etc.). With this
layout `cargo check --locked --all-targets` succeeds without the previous `RuntimeVmRemediationRun`
type mismatches, and future binaries should follow the same pattern when pulling in shared modules.

### Validation harness (`validation: remediation_flow`)

An end-to-end SQLx integration test (`backend/tests/remediation_flow.rs`) now validates the
remediation lifecycle: playbook creation/edit/version guards, queued runs with duplicate protection,
approval transitions, placement gate veto notes, and artifact retrieval. The suite now drives a
multi-tenant chaos matrix (`validation: remediation-chaos-matrix`) that concurrently executes each
scenario across three tenant shards (`tenant-alpha`, `tenant-bravo`, `tenant-charlie`) to ensure
cross-operator isolation and recovery sequencing remain stable:

- `validation:tenant-isolation` – per-tenant run scoping, trust registry tagging, and placement gate
  separation across distinct operators/servers while preventing metadata bleed between shards.
- `validation:concurrent-approvals` – approval version races to ensure stale writes are rejected and
  optimistic locking holds under parallel tenant traffic.
- `validation:executor-outage` – executor unavailability, trust registry failure tagging, and
  scheduler resumption via successful retries (verifying failed/complete statuses and empty queues).

A companion concurrency scenario (`validation: remediation-concurrency`) continues to stress-test
duplicate enqueue races so only a single pending run survives simultaneous submissions. The SSE
verification suite (`validation: remediation-stream:sse-ordering`) now attaches an operator token to
`/api/trust/remediation/stream`, asserting monotonic log sequencing and manifest tag propagation
during the chaos manifest runs.

Execute the harness via:

```bash
DATABASE_URL=postgres://postgres:password@localhost/mcp \
JWT_SECRET=integration-secret \
cargo test --test remediation_flow -- --ignored --nocapture
```

For a fully orchestrated run—including ephemeral Postgres and backend bootstrapping—use the shell
script under `scripts/remediation_harness/` (documented in the harness README). The harness now
emits a JSON manifest enumerating the executed validation tags so dashboards can ingest
`validation:remediation_flow`, `validation:remediation-concurrency`,
`validation:remediation-workspace-draft`, `validation:remediation-workspace-promotion`,
`validation:remediation-workspace-cli`, and the chaos matrix lineage.

### Continuous verification manifests

- Scenario definitions live under `scripts/remediation_harness/scenarios/` and can be authored as
  YAML or JSON. Each document includes a scenario `name`, tracking `tag`, `kind`, and one or more
  tenant shards to exercise. The backend integration test loader resolves the directory via the
  `REM_FABRIC_SCENARIO_DIR` environment variable (defaulting to the repo's harness folder) and will
  fail fast if the directory is empty or missing.
- `backend/tests/remediation_flow.rs` materializes a run matrix from the manifest documents before
  spawning chaos scenarios, ensuring new regression fixtures (for example historical incidents)
  automatically join the verification fabric without code changes.
- `scripts/remediation_harness/run_harness.sh` now aggregates scenario manifests into a machine-verifiable
  JSON artifact containing the SHA-256 for each source document. Dashboards can diff the manifest to
  detect drift, while operators have a single manifest recording the backend database URL, generation
  timestamp, scenario root, and individual scenario descriptors. The harness also records a JSONL SSE
  transcript (captured via `mcpctl remediation watch --json`) so dashboards can replay remediation
  log/status events tagged with the same manifest metadata enforced by the integration suite.
