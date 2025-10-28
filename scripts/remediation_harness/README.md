# Remediation Lifecycle Harness

This harness provisions a disposable Postgres instance, boots the backend API, and executes the
`backend/tests/remediation_flow.rs` integration test to exercise remediation playbook CRUD,
queued runs, approval gating, registry lifecycle transitions, and artifact retrieval.

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
4. Run `cargo test --test remediation_flow -- --ignored --nocapture` against the live database.
5. Tear down the backend process and Postgres container.

Environment variables allow customization:

| Variable | Default | Description |
| --- | --- | --- |
| `HARNESS_POSTGRES_CONTAINER` | `remediation-harness-pg` | Docker container name. |
| `HARNESS_POSTGRES_IMAGE` | `postgres:15-alpine` | Postgres image tag. |
| `HARNESS_POSTGRES_PORT` | `6543` | Host port exposed by Postgres. |
| `HARNESS_PORT` | `38080` | Backend HTTP port. |
| `HARNESS_JWT_SECRET` | `integration-secret` | JWT secret exported to the backend and integration test. |

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
