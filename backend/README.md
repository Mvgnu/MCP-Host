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
`validation:remediation_flow`, `validation:remediation-concurrency`, and the chaos matrix lineage.

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
