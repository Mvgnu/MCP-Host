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
SCENARIO_ROOT="${HARNESS_SCENARIO_ROOT:-${HARNESS_DIR}/scenarios}"
SSE_TRANSCRIPT="${HARNESS_DIR}/remediation_stream.jsonl"

cleanup() {
    set +e
    if [[ -n "${SSE_PID:-}" ]]; then
        kill "${SSE_PID}" >/dev/null 2>&1 || true
        wait "${SSE_PID}" 2>/dev/null || true
    fi
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

TOKEN=$(python3 - <<'PY' "${JWT_SECRET}"
import base64
import hashlib
import hmac
import json
import sys
import time

secret = sys.argv[1].encode()

def encode_segment(document):
    raw = json.dumps(document, separators=(",", ":"), sort_keys=True).encode()
    return base64.urlsafe_b64encode(raw).rstrip(b"=")

header = {"alg": "HS256", "typ": "JWT"}
payload = {"sub": 1, "role": "operator", "exp": int(time.time()) + 3600}
segments = [encode_segment(header), encode_segment(payload)]
signature = hmac.new(secret, b".".join(segments), hashlib.sha256).digest()
segments.append(base64.urlsafe_b64encode(signature).rstrip(b"="))
print(".".join(segment.decode() for segment in segments))
PY
)

export MCP_HOST_URL="http://127.0.0.1:${HARNESS_PORT}"
export MCP_HOST_TOKEN="${TOKEN}"
PYTHONPATH="${REPO_ROOT}/cli" python3 -m mcpctl remediation watch --json >"${SSE_TRANSCRIPT}" 2>&1 &
SSE_PID=$!
sleep 2

echo "[harness] executing remediation lifecycle integration test" >&2
(
    cd "${REPO_ROOT}/backend"
    DATABASE_URL="${DATABASE_URL}" \
    JWT_SECRET="${JWT_SECRET}" \
    cargo test --test remediation_flow -- --ignored --nocapture
)

if [[ -n "${SSE_PID:-}" ]]; then
    kill "${SSE_PID}" >/dev/null 2>&1 || true
    wait "${SSE_PID}" 2>/dev/null || true
fi

echo "[harness] exercising remediation workspace CLI flow" >&2
PYTHONPATH="${REPO_ROOT}/cli" python3 - <<'PY'
import json
import os
import subprocess
import sys

env = os.environ.copy()
base = [sys.executable, "-m", "mcpctl", "remediation", "workspaces"]


def run(args, *, expect_json=True):
    command = base + args
    if expect_json:
        command = command + ["--json"]
    output = subprocess.check_output(command, env=env).decode()
    return json.loads(output) if expect_json else output


def select_revision(envelope, revision_id):
    for item in envelope.get("revisions", []):
        if item.get("revision", {}).get("id") == revision_id:
            return item
    raise AssertionError(f"revision {revision_id} not found")


def parse_automation_rows(output):
    marker = "Automation status:"
    if marker not in output:
        raise AssertionError("missing automation status table")
    table = output.split(marker, 1)[1].strip()
    lines = [line.rstrip() for line in table.splitlines() if line.strip()]
    if len(lines) < 3:
        raise AssertionError("automation table missing rows")
    header = lines[0].split()
    rows = []
    for line in lines[2:]:
        cells = line.split()
        row = {header[idx]: cells[idx] if idx < len(cells) else "" for idx in range(len(header))}
        rows.append(row)
    return rows


records = run(["list"])
if not records:
    raise SystemExit("workspace list returned no records")

records.sort(key=lambda item: item.get("workspace", {}).get("id", 0))
workspace = records[-1]
workspace_id = workspace["workspace"]["id"]
workspace_version = workspace["workspace"]["version"]

details = run(["get", str(workspace_id)])
latest_revision = details["revisions"][0]
latest_revision_id = latest_revision["revision"]["id"]
latest_revision_version = latest_revision["revision"]["version"]
assert latest_revision["gate_summary"].get("schema_status"), "missing gate summary"
assert latest_revision_version >= 1

plan_body = json.dumps({"steps": ["cli-validation"]})
revision_envelope = run(
    [
        "revision",
        "create",
        str(workspace_id),
        "--plan",
        plan_body,
        "--expected-version",
        str(workspace_version),
        "--lineage-label",
        "harness:cli",
        "--metadata",
        json.dumps({"origin": "harness"}),
    ]
)
workspace_version = revision_envelope["workspace"]["version"]
cli_revision = revision_envelope["revisions"][0]
cli_revision_id = cli_revision["revision"]["id"]
cli_revision_version = cli_revision["revision"]["version"]
assert cli_revision["gate_summary"].get("schema_status") == "pending"

schema_envelope = run(
    [
        "revision",
        "schema",
        str(workspace_id),
        str(cli_revision_id),
        "--status",
        "passed",
        "--context",
        json.dumps({"validator": "cli-harness"}),
        "--metadata",
        json.dumps({"token": "cli-schema"}),
        "--version",
        str(cli_revision_version),
    ]
)
cli_revision = select_revision(schema_envelope, cli_revision_id)
cli_revision_version = cli_revision["revision"]["version"]
assert cli_revision["gate_summary"].get("schema_status") == "passed"

policy_envelope = run(
    [
        "revision",
        "policy",
        str(workspace_id),
        str(cli_revision_id),
        "--status",
        "approved",
        "--context",
        json.dumps({"policy": "cli"}),
        "--metadata",
        json.dumps({"ticket": "CLI-1"}),
        "--version",
        str(cli_revision_version),
    ]
)
cli_revision = select_revision(policy_envelope, cli_revision_id)
cli_revision_version = cli_revision["revision"]["version"]
assert cli_revision["gate_summary"].get("policy_status") == "approved"

simulation_envelope = run(
    [
        "revision",
        "simulate",
        str(workspace_id),
        str(cli_revision_id),
        "--simulator",
        "cli-harness",
        "--state",
        "succeeded",
        "--context",
        json.dumps({"source": "cli"}),
        "--diff",
        json.dumps({"delta": 1}),
        "--metadata",
        json.dumps({"transcript": "cli"}),
        "--version",
        str(cli_revision_version),
    ]
)
cli_revision = select_revision(simulation_envelope, cli_revision_id)
cli_revision_version = cli_revision["revision"]["version"]
assert cli_revision["gate_summary"].get("simulation_status") == "succeeded"

promotion_output = run(
    [
        "revision",
        "promote",
        str(workspace_id),
        str(cli_revision_id),
        "--status",
        "completed",
        "--context",
        json.dumps({"lane": "cli", "stage": "promotion"}),
        "--workspace-version",
        str(workspace_version),
        "--version",
        str(cli_revision_version),
        "--note",
        "cli-harness",
    ],
    expect_json=False,
)
automation_rows = parse_automation_rows(promotion_output)
if not any(row.get("gate_lane") == "cli" and row.get("gate_stage") == "promotion" for row in automation_rows):
    raise AssertionError("promotion automation table missing expected lane/stage")

latest_details = run(["get", str(workspace_id)])
workspace_version = latest_details["workspace"]["version"]
cli_revision_final = select_revision(latest_details, cli_revision_id)
cli_revision_version = cli_revision_final["revision"]["version"]
assert cli_revision_final["gate_summary"].get("policy_status") == "approved"
assert cli_revision_final["gate_summary"].get("schema_status") == "passed"
assert cli_revision_final["gate_summary"].get("simulation_status") == "succeeded"
assert cli_revision_final["gate_summary"].get("promotion_status") == "completed"
PY

TIMESTAMP=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
python3 - <<'PY' "${SCENARIO_ROOT}" "${MANIFEST_PATH}" "${DATABASE_URL}" "${TIMESTAMP}" "${SSE_TRANSCRIPT}"
import json
import hashlib
import os
import sys
from pathlib import Path

scenario_root = Path(sys.argv[1])
manifest_path = Path(sys.argv[2])
database_url = sys.argv[3]
timestamp = sys.argv[4]
transcript_path = Path(sys.argv[5])

records = []
if scenario_root.exists():
    for candidate in sorted(scenario_root.rglob('*')):
        if not candidate.is_file():
            continue
        extension = candidate.suffix.lower()
        if extension not in {'.json', '.yaml', '.yml'}:
            continue
        raw = candidate.read_bytes()
        checksum = hashlib.sha256(raw).hexdigest()
        text = raw.decode('utf-8', 'ignore')
        description = ""
        if extension == '.json':
            try:
                payload = json.loads(text)
                description = payload.get('description', "")
            except json.JSONDecodeError:
                description = ""
        else:
            for line in text.splitlines():
                stripped = line.strip()
                if stripped.startswith('description:'):
                    description = stripped.split(':', 1)[1].strip().strip('"')
                    break

        records.append({
            "path": str(candidate.relative_to(scenario_root)),
            "absolute_path": str(candidate),
            "sha256": checksum,
            "format": extension.lstrip('.'),
            "description": description,
        })
else:
    os.makedirs(scenario_root, exist_ok=True)

transcript_record = None
if transcript_path.exists():
    raw = transcript_path.read_bytes()
    transcript_record = {
        "path": str(transcript_path),
        "sha256": hashlib.sha256(raw).hexdigest(),
        "bytes": len(raw),
    }

manifest = {
    "generated_at": timestamp,
    "database_url": database_url,
    "scenario_root": str(scenario_root),
    "scenarios": records,
    "stream_transcript": transcript_record,
    "validation_tags": [
        "validation:remediation_flow",
        "validation:remediation-concurrency",
        "validation:remediation-chaos-matrix",
        "validation:remediation-workspace-draft",
        "validation:remediation-workspace-promotion",
        "validation:remediation-workspace-cli",
    ],
}

manifest_path.write_text(json.dumps(manifest, indent=2))
PY
echo "[harness] wrote manifest to ${MANIFEST_PATH} (scenarios: ${SCENARIO_ROOT})" >&2

echo "[harness] complete" >&2
