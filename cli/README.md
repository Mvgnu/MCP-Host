# MCP Operator Mission-Control CLI

The `mcpctl` tool provides a terminal-first workflow for marketplace inspection, promotion lifecycle management, governance orchestration, and evaluation maintenance. It replaces the former `scripts/mcp_cli.py` helper with a modular package that maps one-to-one onto the backend API surface.

## Installation

```bash
pip install -e ./cli
```

The editable install exposes the `mcpctl` console entry point. Authentication defaults to the `MCP_HOST_TOKEN` environment variable and the host URL defaults to `MCP_HOST_URL` (falling back to `http://localhost:3000`).

## Command groups

### Marketplace

```bash
mcpctl marketplace list
```

Lists marketplace artifacts and their active status. Pass `--json` to receive raw JSON responses.

### Promotions

* `mcpctl promotions tracks list` – enumerate the promotion tracks available to the authenticated operator.
* `mcpctl promotions history [--track-id TRACK] [--manifest DIGEST]` – inspect promotion history.
* `mcpctl promotions schedule TRACK_ID MANIFEST STAGE [--artifact-run-id RUN] [--note NOTE]` – schedule a promotion.
* `mcpctl promotions approve PROMOTION_ID [--note NOTE]` – approve a scheduled promotion.

### Governance

* `mcpctl governance workflows list` – view configured governance workflows.
* `mcpctl governance workflows start WORKFLOW_ID [--artifact-run-id RUN] [--manifest-digest DIGEST] [--context JSON]` – trigger a workflow run.
* `mcpctl governance runs get RUN_ID` – inspect a workflow run’s status.

### Evaluations

* `mcpctl evaluations list` – list evaluation certifications across artifacts.
* `mcpctl evaluations retry EVALUATION_ID` – schedule a certification retry.

### Lifecycle console

* `mcpctl lifecycle list [--lifecycle-state STATE] [--owner-id USER] [--promotion-lane LANE]` – fetch lifecycle console snapshots with promotion automation context, posture verdicts, and recent remediation runs, including duration windows, retry ledgers, manual override provenance, artifact fingerprints, and linked promotion verdicts.
* `mcpctl lifecycle watch [filters]` – subscribe to the lifecycle SSE feed and stream promotion automation deltas, gate verdicts, retry/duration analytics, artifact provenance updates, and heartbeat metadata in real time.

`mcpctl lifecycle list` renders per-run columns for `attempt`, total `retries`, ledger summaries, duration (seconds/ms), override actors, promotion verdict references, artifact fingerprints, trust posture, marketplace readiness, and artifact provenance so operators can pivot between CLI and console views without losing analytics coverage.

### Provider BYOK keys (staging)

* `mcpctl keys register PROVIDER_ID [--alias LABEL] --attestation PATH [--rotation-due RFC3339]` – registers a provider key, hashing the attestation bundle locally, base64-encoding the bundle as a signature, and forwarding both values with an optional rotation deadline to the backend. The endpoint still advertises `501 Not Implemented` while migrations roll out, but successful environments return the persisted record including signature posture metadata.
* `mcpctl keys list PROVIDER_ID` – lists provider key posture records. The command prints a friendly notice when the backend still returns `501`.
* `mcpctl keys rotate PROVIDER_ID KEY_ID --attestation PATH --actor-ref REF` – requests a rotation for an active provider key. The CLI hashes the attestation bundle, forwards a base64 signature, and requires an operator reference for downstream audit trails, matching the backend rotation contract.
* `mcpctl keys bind PROVIDER_ID KEY_ID --type TYPE --target TARGET [--context JSON]` – attaches a provider key to a workload scope (e.g., workspace or runtime), parsing optional JSON context for audit metadata and surfacing the persisted binding record.
* `mcpctl keys bindings PROVIDER_ID KEY_ID [--json]` – lists active bindings for a provider key, including binding type, target identifiers, and creation timestamps for operators reconciling workload coverage.
* Additional subcommands (`approve-rotation`, `watch`) remain scaffolded placeholders while backend orchestration endpoints are staged.

### Policy insights

* `mcpctl policy intelligence SERVER_ID [--json]` – display capability intelligence scores, status, and recent anomaly notes for the specified server.
* `mcpctl policy vm SERVER_ID [--json]` – inspect VM attestation status, isolation tier, and active instance details for confidential workloads.
* `mcpctl policy watch [--server-id ID]` – stream runtime policy events, including BYOK key posture (state, rotation deadlines, veto status) derived from the `key_posture` payload persisted on decisions.

### Trust control plane

* `mcpctl trust registry [--server-id ID] [--lifecycle STATE] [--status STATUS] [--stale|--fresh]` – list the latest registry snapshots with optional filters.
* `mcpctl trust get VM_INSTANCE_ID` – show lifecycle, remediation, and provenance metadata for a specific VM instance.
* `mcpctl trust history VM_INSTANCE_ID [--limit N]` – display recent registry transitions.
* `mcpctl trust transition VM_INSTANCE_ID --status STATUS --lifecycle STATE [options]` – submit a guarded lifecycle transition. Supply `--expected-version` to honour optimistic locking tokens.
* `mcpctl trust watch [--server-id ID] [--lifecycle STATE] [--status STATUS]` – stream live trust registry transitions via SSE.

### Remediation control plane

* `mcpctl remediation playbooks list` – enumerate remediation playbooks, their executor types, and approval/SLA metadata.
* `mcpctl remediation runs list [--instance-id ID] [--status STATUS]` – inspect remediation runs across instances.
* `mcpctl remediation runs enqueue INSTANCE_ID PLAYBOOK [--metadata JSON] [--payload JSON] [--owner USER_ID]` – queue a remediation attempt using the catalogued playbook.
* `mcpctl remediation runs approve RUN_ID --state approved|rejected --version VERSION [--notes TEXT]` – record an approval decision using optimistic locking tokens.
* `mcpctl remediation runs artifacts RUN_ID` – list persisted remediation artifacts (logs, evidence bundles).
* `mcpctl remediation watch [--run-id RUN_ID]` – stream remediation log and status events via SSE.

### Scaffolding helpers

The legacy scaffolding commands remain available under the `scaffold` group:

* `mcpctl scaffold fetch-config SERVER_ID [--json]`
* `mcpctl scaffold gen-python SERVER_ID [--output PATH]`
* `mcpctl scaffold gen-ts SERVER_ID [--output PATH]`
* `mcpctl scaffold create NAME --server-id SERVER_ID`

## Output formats

Every subcommand accepts `--json`. When omitted, the CLI renders simple tables that emphasize workflow status and tier information.

## Testing

Run the integration-style CLI tests with:

```bash
pytest cli/tests/test_cli.py
```

The tests stub the HTTP client to validate serialization, command wiring, and argument parsing without requiring a live backend.
