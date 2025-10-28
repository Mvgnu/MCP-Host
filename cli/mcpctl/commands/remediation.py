"""Remediation control plane commands."""
# key: remediation_cli -> commands

from __future__ import annotations

import json
from argparse import ArgumentParser, _SubParsersAction
from typing import Any, Callable, Dict, Iterable, Optional

from ..client import APIClient
from ..renderers import dumps_json, render_table


def install(
    subparsers: _SubParsersAction[ArgumentParser],
    add_common_arguments: Callable[[ArgumentParser], None],
) -> None:
    parser = subparsers.add_parser("remediation", help="Remediation control plane operations")
    remediation_sub = parser.add_subparsers(dest="remediation_cmd", required=True)

    playbooks_parser = remediation_sub.add_parser("playbooks", help="Manage remediation playbooks")
    playbooks_sub = playbooks_parser.add_subparsers(dest="playbooks_cmd", required=True)

    pb_list = playbooks_sub.add_parser("list", help="List remediation playbooks")
    pb_list.set_defaults(handler=_playbooks_list)
    add_common_arguments(pb_list)

    workspaces_parser = remediation_sub.add_parser(
        "workspaces", help="Manage remediation workspaces"
    )
    workspaces_sub = workspaces_parser.add_subparsers(
        dest="workspaces_cmd", required=True
    )

    ws_list = workspaces_sub.add_parser("list", help="List remediation workspaces")
    ws_list.set_defaults(handler=_workspaces_list)
    add_common_arguments(ws_list)

    ws_get = workspaces_sub.add_parser("get", help="Show workspace details")
    ws_get.add_argument("workspace_id", type=int)
    ws_get.set_defaults(handler=_workspaces_get)
    add_common_arguments(ws_get)

    ws_create = workspaces_sub.add_parser("create", help="Create a workspace draft")
    ws_create.add_argument("workspace_key")
    ws_create.add_argument("display_name")
    ws_create.add_argument("--plan", required=True, help="JSON plan payload")
    ws_create.add_argument("--description")
    ws_create.add_argument("--metadata", help="JSON metadata")
    ws_create.add_argument(
        "--lineage-tag",
        dest="lineage_tags",
        action="append",
        default=[],
        help="Workspace lineage tag",
    )
    ws_create.add_argument(
        "--lineage-label",
        dest="lineage_labels",
        action="append",
        default=[],
        help="Initial revision lineage label",
    )
    ws_create.set_defaults(handler=_workspaces_create)
    add_common_arguments(ws_create)

    revision_parser = workspaces_sub.add_parser(
        "revision", help="Operate on workspace revisions"
    )
    revision_sub = revision_parser.add_subparsers(
        dest="workspace_revision_cmd", required=True
    )

    rev_create = revision_sub.add_parser(
        "create", help="Create a new workspace revision"
    )
    rev_create.add_argument("workspace_id", type=int)
    rev_create.add_argument("--plan", required=True, help="JSON plan payload")
    rev_create.add_argument("--metadata", help="JSON metadata")
    rev_create.add_argument(
        "--expected-version",
        dest="expected_workspace_version",
        type=int,
        required=True,
    )
    rev_create.add_argument(
        "--previous-revision",
        dest="previous_revision_id",
        type=int,
    )
    rev_create.add_argument(
        "--lineage-label",
        dest="lineage_labels",
        action="append",
        default=[],
    )
    rev_create.set_defaults(handler=_workspace_revision_create)
    add_common_arguments(rev_create)

    rev_schema = revision_sub.add_parser(
        "schema", help="Record schema validation outcome"
    )
    rev_schema.add_argument("workspace_id", type=int)
    rev_schema.add_argument("revision_id", type=int)
    rev_schema.add_argument("--status", dest="result_status", required=True)
    rev_schema.add_argument(
        "--error",
        dest="errors",
        action="append",
        default=[],
        help="Validation error",
    )
    rev_schema.add_argument("--context", dest="gate_context", help="JSON gate context")
    rev_schema.add_argument("--metadata", help="JSON metadata")
    rev_schema.add_argument(
        "--version",
        dest="expected_revision_version",
        type=int,
        required=True,
    )
    rev_schema.set_defaults(handler=_workspace_revision_schema)
    add_common_arguments(rev_schema)

    rev_policy = revision_sub.add_parser(
        "policy", help="Record policy feedback for a revision"
    )
    rev_policy.add_argument("workspace_id", type=int)
    rev_policy.add_argument("revision_id", type=int)
    rev_policy.add_argument("--status", dest="policy_status", required=True)
    rev_policy.add_argument(
        "--veto",
        dest="veto_reasons",
        action="append",
        default=[],
        help="Policy veto reason",
    )
    rev_policy.add_argument("--context", dest="gate_context", help="JSON gate context")
    rev_policy.add_argument("--metadata", help="JSON metadata")
    rev_policy.add_argument(
        "--version",
        dest="expected_revision_version",
        type=int,
        required=True,
    )
    rev_policy.set_defaults(handler=_workspace_revision_policy)
    add_common_arguments(rev_policy)

    rev_sim = revision_sub.add_parser(
        "simulate", help="Record sandbox simulation state"
    )
    rev_sim.add_argument("workspace_id", type=int)
    rev_sim.add_argument("revision_id", type=int)
    rev_sim.add_argument("--simulator", dest="simulator_kind", required=True)
    rev_sim.add_argument("--state", dest="execution_state", required=True)
    rev_sim.add_argument("--context", dest="gate_context", help="JSON gate context")
    rev_sim.add_argument("--diff", dest="diff_snapshot", help="JSON diff snapshot")
    rev_sim.add_argument("--metadata", help="JSON metadata")
    rev_sim.add_argument(
        "--version",
        dest="expected_revision_version",
        type=int,
        required=True,
    )
    rev_sim.set_defaults(handler=_workspace_revision_simulation)
    add_common_arguments(rev_sim)

    rev_promote = revision_sub.add_parser(
        "promote", help="Update promotion status for a revision"
    )
    rev_promote.add_argument("workspace_id", type=int)
    rev_promote.add_argument("revision_id", type=int)
    rev_promote.add_argument("--status", dest="promotion_status", required=True)
    rev_promote.add_argument(
        "--note", dest="notes", action="append", default=[], help="Promotion note"
    )
    rev_promote.add_argument(
        "--workspace-version",
        dest="expected_workspace_version",
        type=int,
        required=True,
    )
    rev_promote.add_argument(
        "--version",
        dest="expected_revision_version",
        type=int,
        required=True,
    )
    rev_promote.set_defaults(handler=_workspace_revision_promote)
    add_common_arguments(rev_promote)

    rev_diff = revision_sub.add_parser(
        "diff", help="Show latest sandbox diff for a revision"
    )
    rev_diff.add_argument("workspace_id", type=int)
    rev_diff.add_argument("revision_id", type=int)
    rev_diff.set_defaults(handler=_workspace_revision_diff)
    add_common_arguments(rev_diff)

    runs_parser = remediation_sub.add_parser("runs", help="Inspect remediation runs")
    runs_sub = runs_parser.add_subparsers(dest="runs_cmd", required=True)

    runs_list = runs_sub.add_parser("list", help="List remediation runs")
    runs_list.add_argument("--instance-id", dest="runtime_vm_instance_id", type=int)
    runs_list.add_argument("--status", dest="status")
    runs_list.set_defaults(handler=_runs_list)
    add_common_arguments(runs_list)

    runs_get = runs_sub.add_parser("get", help="Show remediation run details")
    runs_get.add_argument("run_id", type=int)
    runs_get.set_defaults(handler=_runs_get)
    add_common_arguments(runs_get)

    runs_enqueue = runs_sub.add_parser("enqueue", help="Enqueue remediation run for a VM instance")
    runs_enqueue.add_argument("runtime_vm_instance_id", type=int)
    runs_enqueue.add_argument("playbook", help="Playbook key to execute")
    runs_enqueue.add_argument("--metadata", dest="metadata")
    runs_enqueue.add_argument("--payload", dest="automation_payload")
    runs_enqueue.add_argument("--owner", dest="assigned_owner_id", type=int)
    runs_enqueue.set_defaults(handler=_runs_enqueue)
    add_common_arguments(runs_enqueue)

    runs_approve = runs_sub.add_parser("approve", help="Update remediation run approval state")
    runs_approve.add_argument("run_id", type=int)
    runs_approve.add_argument("--state", dest="new_state", required=True)
    runs_approve.add_argument("--notes", dest="approval_notes")
    runs_approve.add_argument("--version", dest="expected_version", type=int, required=True)
    runs_approve.set_defaults(handler=_runs_approve)
    add_common_arguments(runs_approve)

    runs_artifacts = runs_sub.add_parser("artifacts", help="List remediation artifacts for a run")
    runs_artifacts.add_argument("run_id", type=int)
    runs_artifacts.set_defaults(handler=_runs_artifacts)
    add_common_arguments(runs_artifacts)

    watch_parser = remediation_sub.add_parser("watch", help="Stream remediation events")
    watch_parser.add_argument("--run-id", dest="run_id", type=int)
    watch_parser.set_defaults(handler=_watch)
    add_common_arguments(watch_parser)


def _loads_json(value: Optional[str], field: str) -> Optional[Any]:
    if value is None:
        return None
    try:
        return json.loads(value)
    except json.JSONDecodeError as exc:  # pragma: no cover - user input
        raise ValueError(f"Invalid JSON for {field}: {exc}") from exc


def _playbooks_list(client: APIClient, as_json: bool, _: Dict[str, object]) -> None:
    records = client.get("/api/trust/remediation/playbooks")
    if as_json:
        print(dumps_json(records))
        return
    rows = [
        {
            "id": item.get("id"),
            "key": item.get("playbook_key"),
            "executor": item.get("executor_type"),
            "approval": item.get("approval_required"),
            "sla": item.get("sla_duration_seconds"),
        }
        for item in records
        if isinstance(item, dict)
    ]
    columns = ["id", "key", "executor", "approval", "sla"]
    print(render_table(rows, columns))


def _workspaces_list(client: APIClient, as_json: bool, _: Dict[str, object]) -> None:
    records = client.get("/api/trust/remediation/workspaces")
    if as_json:
        print(dumps_json(records))
        return
    rows = []
    for envelope in records:
        if not isinstance(envelope, dict):
            continue
        workspace = envelope.get("workspace", {})
        revisions = envelope.get("revisions", [])
        latest = revisions[0] if revisions else {}
        revision_body = latest.get("revision", {}) if isinstance(latest, dict) else {}
        gate = latest.get("gate_summary", {}) if isinstance(latest, dict) else {}
        rows.append(
            {
                "id": workspace.get("id"),
                "key": workspace.get("workspace_key"),
                "state": workspace.get("lifecycle_state"),
                "revision": revision_body.get("revision_number"),
                "schema": gate.get("schema_status"),
                "policy": gate.get("policy_status"),
                "simulation": gate.get("simulation_status"),
                "promotion": gate.get("promotion_status"),
            }
        )
    columns = ["id", "key", "state", "revision", "schema", "policy", "simulation", "promotion"]
    print(render_table(rows, columns))


def _workspaces_get(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    envelope = client.get(f"/api/trust/remediation/workspaces/{args['workspace_id']}")
    if as_json:
        print(dumps_json(envelope))
        return
    _print_workspace_details(envelope)


def _workspaces_create(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    plan = _loads_json(args["plan"], "plan") or {}
    metadata = _loads_json(args.get("metadata"), "metadata") or {}
    payload: Dict[str, Any] = {
        "workspace_key": args["workspace_key"],
        "display_name": args["display_name"],
        "plan": plan,
        "metadata": metadata,
        "lineage_tags": list(args.get("lineage_tags", [])),
        "lineage_labels": list(args.get("lineage_labels", [])),
    }
    if args.get("description"):
        payload["description"] = args["description"]
    envelope = client.post(
        "/api/trust/remediation/workspaces",
        json_body=payload,
    )
    if as_json:
        print(dumps_json(envelope))
        return
    _print_workspace_details(envelope)


def _workspace_revision_create(
    client: APIClient, as_json: bool, args: Dict[str, object]
) -> None:
    plan = _loads_json(args["plan"], "plan") or {}
    metadata = _loads_json(args.get("metadata"), "metadata") or {}
    payload: Dict[str, Any] = {
        "plan": plan,
        "metadata": metadata,
        "expected_workspace_version": args["expected_workspace_version"],
        "lineage_labels": list(args.get("lineage_labels", [])),
    }
    if args.get("previous_revision_id") is not None:
        payload["previous_revision_id"] = args["previous_revision_id"]
    envelope = client.post(
        f"/api/trust/remediation/workspaces/{args['workspace_id']}/revisions",
        json_body=payload,
    )
    if as_json:
        print(dumps_json(envelope))
        return
    _print_workspace_details(envelope)


def _workspace_revision_schema(
    client: APIClient, as_json: bool, args: Dict[str, object]
) -> None:
    gate_context = _loads_json(args.get("gate_context"), "context") or {}
    metadata = _loads_json(args.get("metadata"), "metadata") or {}
    payload = {
        "result_status": args["result_status"],
        "errors": list(args.get("errors", [])),
        "gate_context": gate_context,
        "metadata": metadata,
        "expected_revision_version": args["expected_revision_version"],
    }
    envelope = client.post(
        f"/api/trust/remediation/workspaces/{args['workspace_id']}/revisions/{args['revision_id']}/schema",
        json_body=payload,
    )
    if as_json:
        print(dumps_json(envelope))
        return
    _print_workspace_details(envelope)


def _workspace_revision_policy(
    client: APIClient, as_json: bool, args: Dict[str, object]
) -> None:
    gate_context = _loads_json(args.get("gate_context"), "context") or {}
    metadata = _loads_json(args.get("metadata"), "metadata") or {}
    payload = {
        "policy_status": args["policy_status"],
        "veto_reasons": list(args.get("veto_reasons", [])),
        "gate_context": gate_context,
        "metadata": metadata,
        "expected_revision_version": args["expected_revision_version"],
    }
    envelope = client.post(
        f"/api/trust/remediation/workspaces/{args['workspace_id']}/revisions/{args['revision_id']}/policy",
        json_body=payload,
    )
    if as_json:
        print(dumps_json(envelope))
        return
    _print_workspace_details(envelope)


def _workspace_revision_simulation(
    client: APIClient, as_json: bool, args: Dict[str, object]
) -> None:
    gate_context = _loads_json(args.get("gate_context"), "context") or {}
    metadata = _loads_json(args.get("metadata"), "metadata") or {}
    diff_snapshot = _loads_json(args.get("diff_snapshot"), "diff")
    payload = {
        "simulator_kind": args["simulator_kind"],
        "execution_state": args["execution_state"],
        "gate_context": gate_context,
        "metadata": metadata,
        "expected_revision_version": args["expected_revision_version"],
    }
    if diff_snapshot is not None:
        payload["diff_snapshot"] = diff_snapshot
    envelope = client.post(
        f"/api/trust/remediation/workspaces/{args['workspace_id']}/revisions/{args['revision_id']}/simulation",
        json_body=payload,
    )
    if as_json:
        print(dumps_json(envelope))
        return
    _print_workspace_details(envelope)


def _workspace_revision_promote(
    client: APIClient, as_json: bool, args: Dict[str, object]
) -> None:
    payload = {
        "promotion_status": args["promotion_status"],
        "notes": list(args.get("notes", [])),
        "expected_workspace_version": args["expected_workspace_version"],
        "expected_revision_version": args["expected_revision_version"],
    }
    envelope = client.post(
        f"/api/trust/remediation/workspaces/{args['workspace_id']}/revisions/{args['revision_id']}/promotion",
        json_body=payload,
    )
    if as_json:
        print(dumps_json(envelope))
        return
    _print_workspace_details(envelope)


def _workspace_revision_diff(
    client: APIClient, as_json: bool, args: Dict[str, object]
) -> None:
    envelope = client.get(f"/api/trust/remediation/workspaces/{args['workspace_id']}")
    revision = _find_revision(envelope, args["revision_id"])
    if revision is None:
        raise ValueError("Revision not found")
    sandbox_runs = revision.get("sandbox_executions", [])
    if not sandbox_runs:
        message = "No sandbox executions recorded"
        if as_json:
            print(dumps_json({"message": message}))
        else:
            print(message)
        return
    latest = sandbox_runs[0]
    if as_json:
        print(dumps_json(latest))
        return
    print(
        f"Simulator {latest.get('simulator_kind')} status {latest.get('execution_state')}"
    )
    if latest.get("diff_snapshot") is not None:
        print(dumps_json(latest.get("diff_snapshot")))
    else:
        print("No diff snapshot available")


def _print_workspace_details(envelope: Dict[str, Any]) -> None:
    workspace = envelope.get("workspace", {})
    summary_row = {
        "id": workspace.get("id"),
        "key": workspace.get("workspace_key"),
        "state": workspace.get("lifecycle_state"),
        "active_revision": workspace.get("active_revision_id"),
        "version": workspace.get("version"),
        "owner": workspace.get("owner_id"),
    }
    print(render_table([summary_row], list(summary_row.keys())))
    revision_rows = []
    for revision in envelope.get("revisions", []):
        if not isinstance(revision, dict):
            continue
        body = revision.get("revision", {})
        gate = revision.get("gate_summary", {})
        revision_rows.append(
            {
                "revision": body.get("revision_number"),
                "schema": gate.get("schema_status"),
                "policy": gate.get("policy_status"),
                "simulation": gate.get("simulation_status"),
                "promotion": gate.get("promotion_status"),
                "veto_reasons": ";".join(
                    gate.get("policy_veto_reasons", []) or []
                ),
                "updated_at": body.get("updated_at"),
            }
        )
    if revision_rows:
        print(render_table(revision_rows, list(revision_rows[0].keys())))


def _find_revision(
    envelope: Dict[str, Any], revision_id: int
) -> Optional[Dict[str, Any]]:
    for revision in envelope.get("revisions", []):
        if not isinstance(revision, dict):
            continue
        body = revision.get("revision")
        if isinstance(body, dict) and body.get("id") == revision_id:
            return revision
    return None


def _runs_list(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    params: Dict[str, Any] = {}
    if args.get("runtime_vm_instance_id") is not None:
        params["runtime_vm_instance_id"] = args["runtime_vm_instance_id"]
    if args.get("status"):
        params["status"] = args["status"]
    records = client.get("/api/trust/remediation/runs", params=params or None)
    if as_json:
        print(dumps_json(records))
        return
    rows = []
    for item in records:
        if not isinstance(item, dict):
            continue
        rows.append(
            {
                "id": item.get("id"),
                "instance": item.get("runtime_vm_instance_id"),
                "playbook": item.get("playbook"),
                "status": item.get("status"),
                "approval": item.get("approval_state"),
                "owner": item.get("assigned_owner_id"),
                "sla_deadline": item.get("sla_deadline"),
                "updated_at": item.get("updated_at"),
            }
        )
    columns = [
        "id",
        "instance",
        "playbook",
        "status",
        "approval",
        "owner",
        "sla_deadline",
        "updated_at",
    ]
    print(render_table(rows, columns))


def _runs_get(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    run = client.get(f"/api/trust/remediation/runs/{args['run_id']}")
    if as_json:
        print(dumps_json(run))
        return
    columns = [
        "id",
        "runtime_vm_instance_id",
        "playbook",
        "status",
        "approval_state",
        "assigned_owner_id",
        "sla_deadline",
        "started_at",
        "completed_at",
        "failure_reason",
        "updated_at",
    ]
    print(render_table([run], columns))


def _runs_enqueue(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    payload = {
        "runtime_vm_instance_id": args["runtime_vm_instance_id"],
        "playbook": args["playbook"],
        "metadata": _loads_json(args.get("metadata"), "metadata") or {},
    }
    automation_payload = _loads_json(args.get("automation_payload"), "payload")
    if automation_payload is not None:
        payload["automation_payload"] = automation_payload
    if args.get("assigned_owner_id") is not None:
        payload["assigned_owner_id"] = args["assigned_owner_id"]

    response = client.post("/api/trust/remediation/runs", json_body=payload)
    if as_json:
        print(dumps_json(response))
        return
    run = response.get("run") if isinstance(response, dict) else None
    if isinstance(run, dict):
        _runs_get(client, False, {"run_id": run.get("id"), "json": False})
    else:
        print("Remediation run enqueued")


def _runs_approve(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    payload = {
        "new_state": args["new_state"],
        "expected_version": args["expected_version"],
    }
    if args.get("approval_notes"):
        payload["approval_notes"] = args["approval_notes"]
    result = client.post(
        f"/api/trust/remediation/runs/{args['run_id']}/approval",
        json_body=payload,
    )
    if as_json:
        print(dumps_json(result))
        return
    columns = ["id", "approval_state", "approval_decided_at", "approval_notes"]
    print(render_table([result], columns))


def _runs_artifacts(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    artifacts = client.get(
        f"/api/trust/remediation/runs/{args['run_id']}/artifacts"
    )
    if as_json:
        print(dumps_json(artifacts))
        return
    rows = []
    for item in artifacts:
        if not isinstance(item, dict):
            continue
        rows.append(
            {
                "id": item.get("id"),
                "type": item.get("artifact_type"),
                "uri": item.get("uri"),
                "created_at": item.get("created_at"),
            }
        )
    print(render_table(rows, ["id", "type", "uri", "created_at"]))


def _watch(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    params: Dict[str, Any] = {}
    if args.get("run_id") is not None:
        params["run_id"] = args["run_id"]
    try:
        for payload in client.stream_sse(
            "/api/trust/remediation/stream", params=params or None
        ):
            if not payload:
                continue
            if as_json:
                print(payload)
                continue
            try:
                event = json.loads(payload)
            except json.JSONDecodeError:
                print(payload)
                continue
            _render_event(event)
    except KeyboardInterrupt:  # pragma: no cover - user initiated
        pass


def _render_event(event: Dict[str, Any]) -> None:
    run_id = event.get("run_id")
    instance = event.get("instance_id")
    if run_id is None or instance is None:
        return
    body = event.get("event")
    if isinstance(body, dict):
        kind = body.get("event") or body.get("type")
    else:
        kind = None
    prefix = f"run {run_id} (instance {instance})"
    policy_feedback = _string_list(event.get("policy_feedback"))
    remediation_gate, accelerator_gates = _policy_gate_summary(event.get("policy_gate"))
    accelerators = _accelerator_summaries(event.get("accelerators"))
    if isinstance(body, dict) and body.get("event") == "log":
        stream = body.get("stream")
        message = body.get("message")
        timestamp = body.get("timestamp")
        print(f"[{timestamp}] {prefix} [{stream}] {message}")
    elif isinstance(body, dict) and body.get("event") == "status":
        if remediation_gate:
            print(f"{prefix} remediation gate -> {', '.join(remediation_gate)}")
        for gate in accelerator_gates:
            hooks = f" hooks={', '.join(gate.hooks)}" if gate.hooks else ""
            reasons = f" reasons={'; '.join(gate.reasons)}" if gate.reasons else ""
            print(
                f"{prefix} accelerator gate {gate.accelerator_id}{hooks}{reasons}"
            )
        if policy_feedback:
            print(f"{prefix} policy feedback -> {', '.join(policy_feedback)}")
        for accelerator in accelerators:
            notes = (
                f" notes={', '.join(accelerator.policy_feedback)}"
                if accelerator.policy_feedback
                else ""
            )
            print(
                f"{prefix} accelerator {accelerator.accelerator_id}"
                f" ({accelerator.accelerator_type}) posture {accelerator.posture}{notes}"
            )
        status = body.get("status")
        failure = body.get("failure_reason") or "-"
        message = body.get("message") or ""
        print(f"{prefix} status -> {status} (failure {failure}) {message}")
    else:
        print(f"{prefix} event {kind or 'unknown'}: {body}")


class _AcceleratorSummary:
    __slots__ = ("accelerator_id", "accelerator_type", "posture", "policy_feedback")

    def __init__(
        self,
        accelerator_id: str,
        accelerator_type: str,
        posture: str,
        policy_feedback: Iterable[str],
    ) -> None:
        self.accelerator_id = accelerator_id
        self.accelerator_type = accelerator_type
        self.posture = posture
        self.policy_feedback = list(policy_feedback)


class _AcceleratorGateSummary:
    __slots__ = ("accelerator_id", "hooks", "reasons")

    def __init__(
        self, accelerator_id: str, hooks: Iterable[str], reasons: Iterable[str]
    ) -> None:
        self.accelerator_id = accelerator_id
        self.hooks = list(hooks)
        self.reasons = list(reasons)


def _string_list(value: Any) -> list[str]:
    if not isinstance(value, list):
        return []
    result: list[str] = []
    for entry in value:
        if isinstance(entry, str):
            normalized = entry.strip()
            if normalized:
                result.append(normalized)
    return result


def _policy_gate_summary(value: Any) -> tuple[list[str], list[_AcceleratorGateSummary]]:
    if not isinstance(value, dict):
        return ([], [])
    remediation_hooks = _string_list(value.get("remediation_hooks"))
    accelerator_gates = _accelerator_gate_summaries(value.get("accelerator_gates"))
    return (remediation_hooks, accelerator_gates)


def _accelerator_summaries(value: Any) -> list[_AcceleratorSummary]:
    if not isinstance(value, list):
        return []
    summaries: list[_AcceleratorSummary] = []
    for entry in value:
        if not isinstance(entry, dict):
            continue
        accelerator_id = str(entry.get("accelerator_id") or entry.get("id") or "unknown")
        accelerator_type = str(entry.get("accelerator_type") or entry.get("kind") or "unknown")
        posture = str(entry.get("posture") or "unknown")
        feedback = _string_list(entry.get("policy_feedback"))
        summaries.append(
            _AcceleratorSummary(accelerator_id, accelerator_type, posture, feedback)
        )
    return summaries


def _accelerator_gate_summaries(value: Any) -> list[_AcceleratorGateSummary]:
    if not isinstance(value, list):
        return []
    summaries: list[_AcceleratorGateSummary] = []
    for entry in value:
        if not isinstance(entry, dict):
            continue
        accelerator_id = entry.get("accelerator_id")
        if not isinstance(accelerator_id, str):
            continue
        hooks = _string_list(entry.get("hooks"))
        reasons = _string_list(entry.get("reasons"))
        if not hooks and not reasons:
            continue
        summaries.append(_AcceleratorGateSummary(accelerator_id, hooks, reasons))
    return summaries
