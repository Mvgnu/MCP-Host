# Remediation Lifecycle Harness

This harness provisions a disposable Postgres instance, boots the backend API, and executes the
`backend/tests/remediation_flow.rs` integration suite to exercise remediation playbook CRUD,
queued runs, approval gating, registry lifecycle transitions, artifact retrieval, and the
multi-tenant chaos matrix scenarios.

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
   covering `validation:remediation_flow`, `validation:remediation-concurrency`, and the
   `validation:remediation-chaos-matrix` tenant/isolation/executor suites.
5. Emit a machine-readable manifest (see below).
6. Tear down the backend process and Postgres container.

Environment variables allow customization:

| Variable | Default | Description |
| --- | --- | --- |
| `HARNESS_POSTGRES_CONTAINER` | `remediation-harness-pg` | Docker container name. |
| `HARNESS_POSTGRES_IMAGE` | `postgres:15-alpine` | Postgres image tag. |
| `HARNESS_POSTGRES_PORT` | `6543` | Host port exposed by Postgres. |
| `HARNESS_PORT` | `38080` | Backend HTTP port. |
| `HARNESS_JWT_SECRET` | `integration-secret` | JWT secret exported to the backend and integration test. |
| `HARNESS_MANIFEST_PATH` | `${HARNESS_DIR}/remediation_harness_manifest.json` | Override manifest output location. |

## Manifest Output

After a successful run the harness writes a JSON manifest enumerating executed scenarios and their
`validation:*` tags. Downstream dashboards can ingest this artifact to confirm multi-tenant chaos
coverage.

```json
{
  "generated_at": "2025-11-29T00:00:00Z",
  "database_url": "postgres://postgres:remediation@127.0.0.1:6543/mcp",
  "scenarios": [
    {"test": "remediation_lifecycle_harness", "tags": ["validation:remediation_flow"]},
    {"test": "remediation_concurrent_enqueue_dedupe", "tags": ["validation:remediation-concurrency"]},
    {
      "test": "remediation_multi_tenant_chaos_matrix",
      "tags": [
        "validation:remediation-chaos-matrix",
        "validation:tenant-isolation",
        "validation:concurrent-approvals",
        "validation:executor-outage"
      ]
    }
  ]
}
```

## SSE and Scheduler Checks

The integration test validates that remediation approval gating blocks placement until the
registry transitions to `restored`. For streaming verification, launch the harness and then run:

```bash
export MCP_HOST_URL=http://127.0.0.1:${HARNESS_PORT:-38080}
export MCP_HOST_TOKEN=$(python - <<'PY'
import json, time
import jwt
claims = {"sub": 1, "role": "operator", "exp": int(time.time()) + 3600}
print(jwt.encode(claims, "${HARNESS_JWT_SECRET:-integration-secret}", algorithm="HS256"))
PY
)
mcpctl remediation watch --run-id <RUN_ID>
```

This streams SSE events from the live backend; replace `<RUN_ID>` with the ID emitted by the test
logs. The optional Python snippet requires the `pyjwt` package. The SSE step is optional but
recommended for manual confirmation that the CLI consumes live remediation events.
