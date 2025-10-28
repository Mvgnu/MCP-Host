# Remediation Lifecycle Harness

This harness provisions a disposable Postgres instance, boots the backend API, and executes the
`backend/tests/remediation_flow.rs` integration suite to exercise remediation playbook CRUD,
queued runs, approval gating, registry lifecycle transitions, artifact retrieval, and the
multi-tenant chaos matrix scenarios that now fan out across three tenant shards in parallel.

## Prerequisites

* Docker (used to launch the ephemeral Postgres container)
* `curl` and `pg_isready`
* Rust toolchain with `cargo`
* Access to this repository's backend crate (the script runs from the repo root)

## Usage

```bash
./scripts/remediation_harness/run_harness.sh
```

The script will:

1. Launch `postgres:15-alpine` on port `6543` (override with `HARNESS_POSTGRES_PORT`).
2. Start the backend on `http://127.0.0.1:38080` with `JWT_SECRET=integration-secret`.
3. Wait for the HTTP health endpoint to respond.
4. Run `cargo test --test remediation_flow -- --ignored --nocapture` against the live database,
   covering `validation:remediation_flow`, `validation:remediation-concurrency`, the new
   `validation:remediation-stream:sse-ordering` verification, and the
   `validation:remediation-chaos-matrix` suites executed concurrently for each tenant shard.
5. Stream remediation SSE events via `mcpctl remediation watch --json`, persisting a JSONL transcript
   tagged with manifest metadata.
6. Aggregate manifest-driven scenarios (YAML/JSON) into a machine-verifiable summary (see below).
7. Tear down the backend process and Postgres container.

Environment variables allow customization:

| Variable | Default | Description |
| --- | --- | --- |
| `HARNESS_POSTGRES_CONTAINER` | `remediation-harness-pg` | Docker container name. |
| `HARNESS_POSTGRES_IMAGE` | `postgres:15-alpine` | Postgres image tag. |
| `HARNESS_POSTGRES_PORT` | `6543` | Host port exposed by Postgres. |
| `HARNESS_PORT` | `38080` | Backend HTTP port. |
| `HARNESS_JWT_SECRET` | `integration-secret` | JWT secret exported to the backend and integration test. |
| `HARNESS_MANIFEST_PATH` | `${HARNESS_DIR}/remediation_harness_manifest.json` | Override manifest output location. |
| `HARNESS_SCENARIO_ROOT` | `${HARNESS_DIR}/scenarios` | Directory scanned for YAML/JSON scenario manifests. |

## Manifest Output

After a successful run the harness writes a JSON manifest summarizing every scenario definition
discovered under `HARNESS_SCENARIO_ROOT`. Each entry includes a stable SHA-256 checksum so
dashboards can diff for drift and operators can trace which manifest drove the run. The backend
integration suite consumes the same directory via `REM_FABRIC_SCENARIO_DIR`, guaranteeing the
orchestrated harness and direct `cargo test` execution stay aligned.

```json
{
  "generated_at": "2025-11-29T00:00:00Z",
  "database_url": "postgres://postgres:remediation@127.0.0.1:6543/mcp",
  "scenario_root": "/workspace/MCP-Host/scripts/remediation_harness/scenarios",
  "scenarios": [
    {
      "path": "chaos-matrix.yaml",
      "absolute_path": "/workspace/MCP-Host/scripts/remediation_harness/scenarios/chaos-matrix.yaml",
      "sha256": "<sha256>",
      "format": "yaml",
      "description": "Baseline remediation chaos matrix"
    },
    {
      "path": "accelerator-posture.yaml",
      "absolute_path": "/workspace/MCP-Host/scripts/remediation_harness/scenarios/accelerator-posture.yaml",
      "sha256": "<sha256>",
      "format": "yaml",
      "description": "Accelerator remediation posture regression"
    },
    {
      "path": "historical-incidents.json",
      "absolute_path": "/workspace/MCP-Host/scripts/remediation_harness/scenarios/historical-incidents.json",
      "sha256": "<sha256>",
      "format": "json",
      "description": "Historical incident regression manifest"
    }
  ],
  "stream_transcript": {
    "path": "/workspace/MCP-Host/scripts/remediation_harness/remediation_stream.jsonl",
    "sha256": "<sha256>",
    "bytes": 2048
  }
}
```

Author new scenario manifests under `scripts/remediation_harness/scenarios/` to extend the fabric.
Each document accepts `name`, `tag`, `kind`, and `tenants` keys; YAML and JSON formats are both
supported. Optional `metadata` blocks are merged into remediation run metadata, enabling rich SSE
payload validation. The backend integration suite fails fast when the directory is empty so operators
know to check out the latest manifests before executing the harness.

### Scenario metadata schema extensions

`metadata` entries now support structured accelerator posture ingestion and policy feedback wiring:

* `policy_feedback`: array of string tags (for example
  `policy_hook:remediation_gate=accelerator-awaiting-attestation`). Values propagate into remediation
  SSE payloads and CLI summaries so operators immediately see which governance hooks fired.
* `policy_gate`: structured remediation gating payload rendered inside SSE/CLI events. The
  `remediation_hooks` array captures `policy_hook:remediation_gate=*` entries while the
  `accelerator_gates` list encodes per-accelerator hooks (`policy_hook:accelerator_gate=*`) alongside
  human-readable gating `reasons`.
* `accelerators`: array of accelerator descriptors with the following fields:
  * `id`: accelerator inventory identifier stored in the new
    `runtime_vm_accelerator_posture` table.
  * `kind`: hardware class (e.g. `nvidia-h100`).
  * `posture`: current remediation posture tag (`trusted`, `quarantined`, etc.).
  * `policy_feedback`: optional string array mirroring placement/policy hooks for the accelerator.
  * `metadata`: arbitrary JSON surfaced through SSE payloads and persisted alongside the posture
    record. Harness fixtures now include `gating_reasons` so verification tooling can validate that
    CLI/REST surfaces explain accelerator veto context.

The accelerator manifest (`accelerator-posture.yaml`) now contains four scenarios:

1. `validation:accelerator-remediation` &mdash; quarantined accelerator awaiting attestation.
2. `validation:accelerator-degraded` &mdash; degraded posture surfacing maintenance guidance.
3. `validation:accelerator-mixed` &mdash; mixed inventory with partial gating.
4. `validation:accelerator-policy-veto` &mdash; governance-driven veto that pairs accelerator and
   remediation hooks.

Use them to exercise degraded health, mixed fleet, and policy-veto pathways inside the continuous
verification fabric.

## SSE and Scheduler Checks

The harness now spawns an operator token, streams `/api/trust/remediation/stream` via
`mcpctl remediation watch --json`, and writes the transcript to
`scripts/remediation_harness/remediation_stream.jsonl`. The integration suite verifies approval
gating (`validation:remediation_flow`), duplicate suppression (`validation:remediation-concurrency`),
manifest-driven chaos scenarios, and SSE ordering/manifest metadata propagation
(`validation:remediation-stream:sse-ordering`). Review the transcript to confirm log sequencing,
status transitions, and manifest tags for accelerator and tenant-focused scenarios without running
additional manual commands.

## Workspace lifecycle validation (in progress)

Workspace lifecycle APIs are now live in the backend (`/api/trust/remediation/workspaces/*`) and the
CLI (`mcpctl remediation workspaces`). Upcoming harness iterations will stitch the following phases
into the chaos fabric so remediation plans can be rehearsed before production promotion:

1. **Draft creation:** call `mcpctl remediation workspaces create` with manifest-driven plan JSON to
   persist a workspace and seed revision `1`.
2. **Revision iteration:** replay harness manifests through
   `mcpctl remediation workspaces revision create` to simulate operators uploading revised plans,
   including lineage labels that tie revisions back to chaos manifests.
3. **Validation and policy feedback:** issue `revision schema` / `revision policy` commands to record
   gate outcomes, ensuring SSE payloads and REST envelopes surface structured veto reasons.
4. **Simulation playback:** apply `revision simulate` with diff snapshots from chaos scenarios so the
   harness can assert sandbox parity before promotion.
5. **Promotion gates:** finalize flows with `revision promote`, verifying optimistic locking tokens
   (`--workspace-version`, `--version`) and policy veto propagation.

The current harness documentation tracks these tasks as open follow-ups; once implemented, expect
new validation tags (for example `validation:remediation-workspace-draft`) and transcript checks that
confirm gate context flows from harness manifests through backend envelopes and CLI summaries.
