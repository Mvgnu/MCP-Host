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


class FakeClient:
    """Test double for :class:`mcpctl.client.APIClient`."""

    responses: Dict[Tuple[str, str], Any] = {}
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


@pytest.fixture(autouse=True)
def _reset_fake_client(monkeypatch: pytest.MonkeyPatch) -> None:
    FakeClient.responses = {}
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
