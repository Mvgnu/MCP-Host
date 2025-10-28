#!/usr/bin/env bash
set -euo pipefail

# key: validation -> remediation-flow-script
HARNESS_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd "${HARNESS_DIR}/../.." && pwd)
POSTGRES_CONTAINER="${HARNESS_POSTGRES_CONTAINER:-remediation-harness-pg}"
POSTGRES_IMAGE="${HARNESS_POSTGRES_IMAGE:-postgres:15-alpine}"
POSTGRES_PORT="${HARNESS_POSTGRES_PORT:-6543}"
HARNESS_PORT="${HARNESS_PORT:-38080}"
JWT_SECRET="${HARNESS_JWT_SECRET:-integration-secret}"
DATABASE_URL="postgres://postgres:remediation@127.0.0.1:${POSTGRES_PORT}/mcp"
BACKEND_LOG="${HARNESS_DIR}/backend.log"
MANIFEST_PATH="${HARNESS_MANIFEST_PATH:-${HARNESS_DIR}/remediation_harness_manifest.json}"

cleanup() {
    set +e
    if [[ -n "${BACKEND_PID:-}" ]]; then
        kill "${BACKEND_PID}" >/dev/null 2>&1 || true
        wait "${BACKEND_PID}" 2>/dev/null || true
    fi
    if docker ps --format '{{.Names}}' | grep -q "^${POSTGRES_CONTAINER}$"; then
        docker stop "${POSTGRES_CONTAINER}" >/dev/null
    fi
    rm -f "${BACKEND_LOG}"
}
trap cleanup EXIT

echo "[harness] starting postgres container ${POSTGRES_CONTAINER} on port ${POSTGRES_PORT}" >&2
if docker ps --format '{{.Names}}' | grep -q "^${POSTGRES_CONTAINER}$"; then
    echo "[harness] existing container detected; stopping" >&2
    docker stop "${POSTGRES_CONTAINER}" >/dev/null
fi

docker run \
    --rm \
    --name "${POSTGRES_CONTAINER}" \
    -e POSTGRES_PASSWORD=remediation \
    -e POSTGRES_DB=mcp \
    -p "${POSTGRES_PORT}:5432" \
    -d "${POSTGRES_IMAGE}" >/dev/null

echo "[harness] waiting for postgres to become ready" >&2
for _ in {1..30}; do
    if pg_isready -h 127.0.0.1 -p "${POSTGRES_PORT}" -d mcp >/dev/null 2>&1; then
        break
    fi
    sleep 1
done
if ! pg_isready -h 127.0.0.1 -p "${POSTGRES_PORT}" -d mcp >/dev/null 2>&1; then
    echo "[harness] postgres failed to start" >&2
    exit 1
fi

echo "[harness] launching backend on http://127.0.0.1:${HARNESS_PORT}" >&2
(
    cd "${REPO_ROOT}/backend"
    BIND_ADDRESS=127.0.0.1 \
    BIND_PORT="${HARNESS_PORT}" \
    DATABASE_URL="${DATABASE_URL}" \
    JWT_SECRET="${JWT_SECRET}" \
    cargo run --quiet >"${BACKEND_LOG}" 2>&1
) &
BACKEND_PID=$!

for _ in {1..60}; do
    if curl -sf "http://127.0.0.1:${HARNESS_PORT}/" >/dev/null; then
        break
    fi
    if ! kill -0 "${BACKEND_PID}" >/dev/null 2>&1; then
        echo "[harness] backend exited unexpectedly" >&2
        cat "${BACKEND_LOG}" >&2 || true
        exit 1
    fi
    sleep 1
done

if ! curl -sf "http://127.0.0.1:${HARNESS_PORT}/" >/dev/null; then
    echo "[harness] backend did not become ready" >&2
    cat "${BACKEND_LOG}" >&2 || true
    exit 1
fi

echo "[harness] executing remediation lifecycle integration test" >&2
(
    cd "${REPO_ROOT}/backend"
    DATABASE_URL="${DATABASE_URL}" \
    JWT_SECRET="${JWT_SECRET}" \
    cargo test --test remediation_flow -- --ignored --nocapture
)

TIMESTAMP=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
cat >"${MANIFEST_PATH}" <<EOF
{
  "generated_at": "${TIMESTAMP}",
  "database_url": "${DATABASE_URL}",
  "scenarios": [
    {
      "test": "remediation_lifecycle_harness",
      "tags": ["validation:remediation_flow"]
    },
    {
      "test": "remediation_concurrent_enqueue_dedupe",
      "tags": ["validation:remediation-concurrency"]
    },
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
EOF
echo "[harness] wrote manifest to ${MANIFEST_PATH}" >&2

echo "[harness] complete" >&2
