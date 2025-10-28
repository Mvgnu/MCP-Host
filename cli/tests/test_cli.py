"""Integration-style tests for the mission-control CLI."""
# key: operator-cli -> tests

from __future__ import annotations

import json
import sys
import types
from pathlib import Path
from typing import Any, Dict, Tuple

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))


class _StubSession:
    def request(self, *_: Any, **__: Any) -> None:  # pragma: no cover - unused safety
        raise RuntimeError("requests is not available in test stub")


class _StubResponse:  # pragma: no cover - not used in tests
    status_code = 200
    content = b""

    def json(self) -> Dict[str, Any]:
        return {}


requests_stub = types.SimpleNamespace(Session=_StubSession, Response=_StubResponse)
sys.modules.setdefault("requests", requests_stub)

import pytest

from mcpctl import cli as cli_module
from mcpctl.commands import _render_trust_event
from mcpctl.commands.remediation import _render_event as _render_remediation_event


class FakeClient:
    """Test double for :class:`mcpctl.client.APIClient`."""

    responses: Dict[Tuple[str, str], Any] = {}
    streams: Dict[str, list[str]] = {}
    calls: list[Tuple[str, str, Dict[str, Any]]] = []

    def __init__(self, base_url: str, token: str | None, timeout: int) -> None:  # pragma: no cover - args validated by CLI
        self.base_url = base_url
        self.token = token
        self.timeout = timeout

    def get(self, path: str, params: Dict[str, Any] | None = None) -> Any:
        FakeClient.calls.append(("GET", path, params or {}))
        return FakeClient.responses.get(("GET", path), [])

    def post(
        self,
        path: str,
        *,
        json_body: Dict[str, Any] | None = None,
        params: Dict[str, Any] | None = None,
    ) -> Any:
        FakeClient.calls.append(("POST", path, json_body or {}))
        return FakeClient.responses.get(("POST", path), {})

    def patch(
        self,
        path: str,
        *,
        json: Dict[str, Any] | None = None,
        params: Dict[str, Any] | None = None,
    ) -> Any:
        FakeClient.calls.append(("PATCH", path, json or {}))
        return FakeClient.responses.get(("PATCH", path), {})

    def stream_sse(
        self,
        path: str,
        params: Dict[str, Any] | None = None,
    ) -> list[str]:
        FakeClient.calls.append(("STREAM", path, params or {}))
        return list(FakeClient.streams.get(path, []))


@pytest.fixture(autouse=True)
def _reset_fake_client(monkeypatch: pytest.MonkeyPatch) -> None:
    FakeClient.responses = {}
    FakeClient.streams = {}
    FakeClient.calls = []
    monkeypatch.setattr(cli_module, "APIClient", FakeClient)


def test_marketplace_list_outputs_table(capsys: pytest.CaptureFixture[str]) -> None:
    FakeClient.responses[("GET", "/api/marketplace")] = [
        {"id": 1, "name": "Alpha", "tier": "dev", "status": "active"},
        {"id": 2, "name": "Beta", "tier": "prod", "status": "paused"},
    ]

    cli_module.main(["marketplace", "list"])
    captured = capsys.readouterr().out.strip()
    assert "id" in captured.splitlines()[0]
    assert "Alpha" in captured
    assert "Beta" in captured


def test_promotions_schedule_returns_json(capsys: pytest.CaptureFixture[str]) -> None:
    FakeClient.responses[("POST", "/api/promotions/schedule")] = {
        "id": 22,
        "track_name": "production",
        "stage": "prod",
        "status": "scheduled",
    }

    cli_module.main(
        [
            "promotions",
            "schedule",
            "4",
            "sha256:123",
            "production",
            "--json",
            "--note",
            "ready",
        ]
    )
    captured = capsys.readouterr().out.strip()
    payload = json.loads(captured)
    assert payload["id"] == 22
    assert FakeClient.calls[-1][2]["notes"] == ["ready"]


def test_governance_start_parses_context() -> None:
    FakeClient.responses[("POST", "/api/governance/workflows/7/runs")] = {
        "id": 100,
        "workflow_id": 7,
        "status": "running",
        "created_at": "2024-01-01T00:00:00Z",
    }

    exit_code = cli_module.main(
        [
            "governance",
            "workflows",
            "start",
            "7",
            "--context",
            '{"initiator": "ops"}',
        ]
    )
    assert exit_code == 0
    method, path, payload = FakeClient.calls[-1]
    assert method == "POST"
    assert path == "/api/governance/workflows/7/runs"
    assert payload["context"] == {"initiator": "ops"}


def test_remediation_workspace_list_outputs_summary(
    capsys: pytest.CaptureFixture[str],
) -> None:
    envelope = {
        "workspace": {
            "id": 5,
            "workspace_key": "trust-hardening",
            "lifecycle_state": "draft",
            "active_revision_id": 12,
            "version": 2,
            "owner_id": 7,
        },
        "revisions": [
            {
                "revision": {
                    "id": 12,
                    "revision_number": 3,
                    "updated_at": "2025-12-02T00:00:00Z",
                },
                "gate_summary": {
                    "schema_status": "succeeded",
                    "policy_status": "approved",
                    "simulation_status": "succeeded",
                    "promotion_status": "pending",
                    "policy_veto_reasons": [],
                },
                "sandbox_executions": [],
                "validation_snapshots": [],
            }
        ],
    }

    FakeClient.responses[("GET", "/api/trust/remediation/workspaces")] = [envelope]

    cli_module.main(["remediation", "workspaces", "list"])
    output = capsys.readouterr().out
    assert "trust-hardening" in output
    assert "succeeded" in output
    assert FakeClient.calls[-1] == ("GET", "/api/trust/remediation/workspaces", {})


def test_remediation_workspace_revision_diff_json(
    capsys: pytest.CaptureFixture[str],
) -> None:
    envelope = {
        "workspace": {
            "id": 5,
            "workspace_key": "trust-hardening",
            "lifecycle_state": "draft",
            "active_revision_id": 12,
            "version": 2,
            "owner_id": 7,
        },
        "revisions": [
            {
                "revision": {
                    "id": 12,
                    "revision_number": 3,
                    "updated_at": "2025-12-02T00:00:00Z",
                },
                "gate_summary": {
                    "schema_status": "succeeded",
                    "policy_status": "approved",
                    "simulation_status": "succeeded",
                    "promotion_status": "pending",
                    "policy_veto_reasons": [],
                },
                "sandbox_executions": [
                    {
                        "id": 90,
                        "simulator_kind": "staging",
                        "execution_state": "succeeded",
                        "diff_snapshot": {"delta": "ok"},
                    }
                ],
                "validation_snapshots": [],
            }
        ],
    }

    FakeClient.responses[("GET", "/api/trust/remediation/workspaces/5")] = envelope

    cli_module.main(
        [
            "remediation",
            "workspaces",
            "revision",
            "diff",
            "5",
            "12",
            "--json",
        ]
    )
    payload = json.loads(capsys.readouterr().out)
    assert payload["simulator_kind"] == "staging"
    assert payload["execution_state"] == "succeeded"
    assert FakeClient.calls[-1] == ("GET", "/api/trust/remediation/workspaces/5", {})


def test_remediation_promotion_prints_automation_summary(
    capsys: pytest.CaptureFixture[str],
) -> None:
    envelope = {
        "workspace": {
            "id": 7,
            "workspace_key": "workspace.multi",
            "lifecycle_state": "promoted",
            "active_revision_id": 11,
            "version": 4,
            "owner_id": 3,
        },
        "revisions": [
            {
                "revision": {
                    "id": 11,
                    "revision_number": 3,
                    "updated_at": "2025-12-03T00:00:00Z",
                },
                "gate_summary": {
                    "schema_status": "passed",
                    "policy_status": "approved",
                    "simulation_status": "succeeded",
                    "promotion_status": "completed",
                    "policy_veto_reasons": [],
                },
                "sandbox_executions": [],
                "validation_snapshots": [],
            }
        ],
    }
    runs = [
        {
            "id": 55,
            "workspace_id": 7,
            "workspace_revision_id": 11,
            "runtime_vm_instance_id": 202,
            "status": "pending",
            "approval_state": "auto-approved",
            "playbook": "vm.restart",
            "promotion_gate_context": {"lane": "cli", "stage": "promotion"},
            "automation_payload": {"kind": "direct"},
            "metadata": {
                "target": {"trust_posture": "quarantined"},
                "promotion": {"notes": ["cli-harness"]},
            },
            "updated_at": "2025-12-03T00:00:00Z",
        }
    ]

    envelope["promotion_runs"] = runs

    FakeClient.responses[(
        "POST",
        "/api/trust/remediation/workspaces/7/revisions/11/promotion",
    )] = envelope

    cli_module.main(
        [
            "remediation",
            "workspaces",
            "revision",
            "promote",
            "7",
            "11",
            "--status",
            "completed",
            "--workspace-version",
            "3",
            "--version",
            "2",
        ]
    )

    output = capsys.readouterr().out
    assert "Automation status:" in output
    assert "cli" in output
    assert "promotion" in output
    assert "quarantined" in output
    assert "direct" in output
    assert len(FakeClient.calls) == 1
    method, path, params = FakeClient.calls[0]
    assert (method, path) == (
        "POST",
        "/api/trust/remediation/workspaces/7/revisions/11/promotion",
    )


def test_policy_intelligence_displays_scores(
    capsys: pytest.CaptureFixture[str],
) -> None:
    FakeClient.responses[("GET", "/api/intelligence/servers/5/scores")] = [
        {
            "capability": "runtime",
            "score": 82.5,
            "status": "healthy",
            "backend": "docker",
            "tier": "silver:Router",
            "last_observed_at": "2025-11-10T00:00:00Z",
            "notes": ["stable"],
        },
        {
            "capability": "image-build",
            "score": 58.0,
            "status": "warning",
            "backend": "docker",
            "tier": "silver:Router",
            "last_observed_at": "2025-11-10T00:00:00Z",
            "notes": ["credential-check"],
        },
    ]

    cli_module.main(["policy", "intelligence", "5"])
    captured = capsys.readouterr().out
    assert "runtime" in captured
    assert "image-build" in captured
    assert "82.5" in captured


def test_policy_vm_runtime_outputs_summary(
    capsys: pytest.CaptureFixture[str],
) -> None:
    FakeClient.responses[("GET", "/api/servers/7/vm")] = {
        "latest_status": "trusted",
        "last_updated_at": "2025-11-16T12:00:00Z",
        "active_instance_id": "vm-alpha",
        "instances": [
            {
                "instance_id": "vm-alpha",
                "attestation_status": "trusted",
                "isolation_tier": "coco",
                "updated_at": "2025-11-16T12:00:00Z",
            },
            {
                "instance_id": "vm-beta",
                "attestation_status": "untrusted",
                "isolation_tier": None,
                "updated_at": "2025-11-15T09:30:00Z",
            },
        ],
    }

    cli_module.main(["policy", "vm", "7"])
    captured = capsys.readouterr().out
    assert "vm-alpha" in captured
    assert "trusted" in captured


def test_policy_watch_renders_stream(capsys: pytest.CaptureFixture[str]) -> None:
    FakeClient.streams["/api/policy/stream"] = [
        json.dumps(
            {
                "server_id": 9,
                "timestamp": "2025-11-17T12:34:56Z",
                "type": "decision",
                "backend": "virtual-machine",
                "candidate_backend": "virtual-machine",
                "attestation_status": "trusted",
                "evaluation_required": True,
                "notes": ["vm:attestation:trusted"],
            }
        ),
        json.dumps(
            {
                "server_id": 9,
                "timestamp": "2025-11-17T12:36:00Z",
                "type": "attestation",
                "attestation_status": "untrusted",
                "instance_id": "vm-alpha",
                "notes": ["attestation:stale", "attestation:measurement:bad"],
            }
        ),
    ]

    cli_module.main(["policy", "watch"])
    output = capsys.readouterr().out
    assert "server 9 DECISION" in output
    assert "attestation trusted" in output
    assert "attestation untrusted" in output
    assert FakeClient.calls[-1][0] == "STREAM"
    assert "Latest posture: trusted" in output
    assert "Active instance: vm-alpha" in output


def test_trust_registry_lists_entries(capsys: pytest.CaptureFixture[str]) -> None:
    FakeClient.responses[("GET", "/api/trust/registry")] = [
        {
            "server_name": "alpha",
            "server_id": 9,
            "instance_id": "vm-alpha",
            "vm_instance_id": 101,
            "attestation_status": "untrusted",
            "lifecycle_state": "quarantined",
            "remediation_state": "remediation:pending",
            "remediation_attempts": 2,
            "stale": True,
            "updated_at": "2025-11-21T10:00:00Z",
        }
    ]

    cli_module.main(["trust", "registry"])
    output = capsys.readouterr().out
    assert "vm-alpha" in output
    assert "quarantined" in output


def test_render_trust_event_handles_missing_fields() -> None:
    event = {
        "server_id": 7,
        "vm_instance_id": 55,
        "triggered_at": "2025-11-21T12:00:00Z",
        "attestation_status": "trusted",
        "previous_attestation_status": "untrusted",
        "lifecycle_state": "restored",
        "previous_lifecycle_state": "remediating",
        "stale": False,
        "remediation_attempts": None,
        "transition_reason": None,
        "version": 3,
    }

    rendered = _render_trust_event(event)
    assert "server 7" in rendered
    assert "status untrusted -> trusted" in rendered
    assert "restored" in rendered
    assert "attempts -" in rendered
    assert rendered.endswith("v3")


def test_evaluations_plan_overrides_payload() -> None:
    FakeClient.responses[("PATCH", "/api/evaluations/42/status")] = {
        "id": 42,
        "status": "pending",
        "next_refresh_at": "2024-01-01T00:00:00+00:00",
    }

    exit_code = cli_module.main(
        [
            "evaluations",
            "plan",
            "42",
            "--cadence-seconds",
            "3600",
            "--next-refresh",
            "2024-01-01T00:00:00Z",
            "--note",
            "review",
        ]
    )

    assert exit_code == 0
    method, path, payload = FakeClient.calls[-1]
    assert method == "PATCH"
    assert path == "/api/evaluations/42/status"
    assert payload["refresh_cadence_seconds"] == 3600


def test_remediation_render_event_includes_policy_feedback(
    capsys: pytest.CaptureFixture[str],
) -> None:
    event = {
        "run_id": 42,
        "instance_id": 1001,
        "policy_feedback": ["policy_hook:remediation_gate=accelerator"],
        "policy_gate": {
            "remediation_hooks": ["policy_hook:remediation_gate=accelerator"],
            "accelerator_gates": [
                {
                    "accelerator_id": "accel-1",
                    "hooks": ["policy_hook:accelerator_gate=awaiting-attestation"],
                    "reasons": ["pending attestation"],
                }
            ],
        },
        "accelerators": [
            {
                "accelerator_id": "accel-1",
                "accelerator_type": "nvidia-h100",
                "posture": "quarantined",
                "policy_feedback": ["accelerator:requires-attestation"],
            }
        ],
        "event": {
            "event": "status",
            "status": "completed",
            "failure_reason": None,
            "message": "automation completed",
        },
    }

    _render_remediation_event(event)
    output = capsys.readouterr().out.strip().splitlines()
    assert any("remediation gate" in line for line in output)
    assert any("accelerator gate accel-1" in line for line in output)
    assert any("policy feedback" in line for line in output)
    assert any("accelerator accel-1" in line for line in output)
    assert any("status -> completed" in line for line in output)
