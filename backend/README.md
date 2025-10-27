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

- **Scheduler:** `evaluations::scheduler` now promotes distrusted instances into the `remediating` lifecycle state while cancelling refresh jobs, preserving fallback timestamps, and keeping optimistic registry versions in sync. Trusted states resume the cadence automatically when restored.
- **Trust notifications:** `trust::spawn_trust_listener` subscribes to the `runtime_vm_trust_transition` channel, recalculates evaluation plans in real time, forwards lifecycle/provenance metadata to SSE clients, and triggers intelligence recomputes whenever posture changes arrive from Postgres.
- **Policy engine:** placement decisions hydrate the latest trust event via `runtime_vm_trust_history`, emit `vm:trust-*` notes for lifecycle, remediation counts, and provenance, and rely on registry snapshots when determining backend overrides.
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
