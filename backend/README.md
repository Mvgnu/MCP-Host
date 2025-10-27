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

The VM runtime now persists posture transitions in a dedicated `runtime_vm_trust_history` table. Each attestation outcome recorded through `policy::trust::persist_vm_attestation_outcome` emits a `pg_notify` event, appends history rows, and updates derived lineage columns on `evaluation_certifications` (`last_attestation_status`, `fallback_launched_at`, `remediation_attempts`). These updates keep evaluation refresh scheduling and intelligence scoring aligned with the active trust posture.

Key integration points:

- **Scheduler:** `evaluations::scheduler` skips refreshes when the latest attestation status is `untrusted`, appends governance notes, and preserves fallback timestamps so operators can audit paused evidence. Trusted states resume the cadence automatically.
- **Trust notifications:** `trust::spawn_trust_listener` subscribes to the `runtime_vm_trust_transition` channel, recalculates evaluation plans in real time, and triggers intelligence recomputes whenever posture changes arrive from Postgres.
- **Policy engine:** placement decisions hydrate the latest trust event via `runtime_vm_trust_history` and emit `vm:trust-event` notes, enabling downstream scoring and telemetry to reason about posture transitions and remediation state.
- **Operator tooling:** `/api/evaluations` and the CLI surface lineage fields for trust, fallback attempts, and remediation counts so consoles can highlight blocked evidence and remediation activity.
- **Intelligence scoring:** scoring logic folds trust transitions, remediation attempts, and transition reasons into capability notes and evidence payloads. Servers with degraded posture incur score penalties proportional to remediation churn.

Refer to `progress.md` for the operational rollout plan covering notification listeners, CLI affordances, and intelligence feedback loops built on top of the trust registry.
