"""Command registration for the mission-control CLI."""
# key: operator-cli -> command-registry

from __future__ import annotations

import json
from argparse import ArgumentParser, _SubParsersAction
from pathlib import Path
from typing import Any, Callable, Dict, Iterable

import sys

from ..client import APIClient, APIError
from ..renderers import dumps_json, render_table
from . import billing as billing_commands
from . import evaluations as evaluations_commands
from . import keys as keys_commands
from . import lifecycle as lifecycle_commands
from . import remediation as remediation_commands
from . import vector_dbs as vector_db_commands

_RESET = "\033[0m"
_GREEN = "\033[32m"
_YELLOW = "\033[33m"
_RED = "\033[31m"
_CYAN = "\033[36m"

CommandFn = Callable[[APIClient, bool, Dict[str, object]], None]


def _add_common_arguments(parser: ArgumentParser) -> None:
    parser.add_argument("--json", action="store_true", help="Render output as JSON")


def install_marketplace(subparsers: _SubParsersAction[ArgumentParser]) -> None:
    parser = subparsers.add_parser("marketplace", help="Marketplace operations")
    marketplace_sub = parser.add_subparsers(dest="marketplace_cmd", required=True)

    list_parser = marketplace_sub.add_parser("list", help="List marketplace offerings")
    list_parser.set_defaults(handler=_marketplace_list)
    _add_common_arguments(list_parser)

    watch_parser = marketplace_sub.add_parser(
        "watch", help="Stream provider marketplace events in real time"
    )
    watch_parser.add_argument("provider_id", help="Provider identifier (UUID)")
    watch_parser.add_argument(
        "--max-events",
        dest="max_events",
        type=int,
        default=0,
        help="Stop after emitting N events (0 = infinite)",
    )
    watch_parser.set_defaults(handler=_marketplace_watch)
    _add_common_arguments(watch_parser)

    submissions_parser = marketplace_sub.add_parser(
        "submissions", help="Manage provider marketplace submissions"
    )
    submissions_sub = submissions_parser.add_subparsers(
        dest="marketplace_submissions_cmd", required=True
    )

    submissions_list = submissions_sub.add_parser(
        "list", help="List submissions and evaluation summaries for a provider"
    )
    submissions_list.add_argument("provider_id", help="Provider identifier (UUID)")
    submissions_list.set_defaults(handler=_marketplace_submissions_list)
    _add_common_arguments(submissions_list)

    submissions_create = submissions_sub.add_parser(
        "create", help="Create a new provider submission"
    )
    submissions_create.add_argument("provider_id", help="Provider identifier (UUID)")
    submissions_create.add_argument(
        "--tier", required=True, help="Marketplace tier for the submission"
    )
    submissions_create.add_argument(
        "--manifest-uri",
        required=True,
        dest="manifest_uri",
        help="Artifact manifest URI (e.g. registry path)",
    )
    submissions_create.add_argument(
        "--artifact-digest", dest="artifact_digest", help="Optional manifest digest"
    )
    submissions_create.add_argument(
        "--release-notes",
        dest="release_notes",
        help="Inline release notes to attach to the submission",
    )
    submissions_create.add_argument(
        "--release-notes-file",
        dest="release_notes_file",
        help="Path to a file containing release notes",
    )
    submissions_create.add_argument(
        "--metadata",
        help="Additional submission metadata as JSON (e.g. {'git': {...}})",
    )
    submissions_create.set_defaults(handler=_marketplace_submissions_create)
    _add_common_arguments(submissions_create)

    evaluations_parser = marketplace_sub.add_parser(
        "evaluations", help="Manage submission evaluation runs"
    )
    evaluations_sub = evaluations_parser.add_subparsers(
        dest="marketplace_evaluations_cmd", required=True
    )

    evaluations_list = evaluations_sub.add_parser(
        "list", help="List evaluations for a provider or specific submission"
    )
    evaluations_list.add_argument("provider_id", help="Provider identifier (UUID)")
    evaluations_list.add_argument(
        "--submission-id",
        dest="submission_id",
        help="Filter evaluations to a specific submission",
    )
    evaluations_list.set_defaults(handler=_marketplace_evaluations_list)
    _add_common_arguments(evaluations_list)

    evaluations_start = evaluations_sub.add_parser(
        "start", help="Start a new evaluation run for a submission"
    )
    evaluations_start.add_argument("provider_id", help="Provider identifier (UUID)")
    evaluations_start.add_argument("submission_id", help="Submission identifier (UUID)")
    evaluations_start.add_argument(
        "evaluation_type", help="Evaluation type identifier (e.g. security)"
    )
    evaluations_start.add_argument(
        "--status", help="Optional evaluation status (defaults to running)"
    )
    evaluations_start.add_argument(
        "--evaluator-ref",
        dest="evaluator_ref",
        help="Optional evaluator reference for audit trail",
    )
    evaluations_start.add_argument(
        "--result", help="Optional JSON payload containing evaluation metadata"
    )
    evaluations_start.set_defaults(handler=_marketplace_evaluations_start)
    _add_common_arguments(evaluations_start)

    evaluations_transition = evaluations_sub.add_parser(
        "transition", help="Transition an evaluation run"
    )
    evaluations_transition.add_argument("provider_id", help="Provider identifier (UUID)")
    evaluations_transition.add_argument(
        "evaluation_id", help="Evaluation identifier (UUID)"
    )
    evaluations_transition.add_argument(
        "--status",
        required=True,
        help="New evaluation status (e.g. completed, failed)",
    )
    evaluations_transition.add_argument(
        "--completed-at",
        dest="completed_at",
        help="Completion timestamp (RFC3339)",
    )
    evaluations_transition.add_argument(
        "--result",
        help="Optional JSON payload describing evaluation results",
    )
    evaluations_transition.set_defaults(handler=_marketplace_evaluations_transition)
    _add_common_arguments(evaluations_transition)

    promotions_parser = marketplace_sub.add_parser(
        "promotions", help="Manage provider promotion gates"
    )
    promotions_sub = promotions_parser.add_subparsers(
        dest="marketplace_promotions_cmd", required=True
    )

    promotions_create = promotions_sub.add_parser(
        "create", help="Create a promotion gate for an evaluation"
    )
    promotions_create.add_argument("provider_id", help="Provider identifier (UUID)")
    promotions_create.add_argument("evaluation_id", help="Evaluation identifier (UUID)")
    promotions_create.add_argument("--gate", required=True, help="Promotion gate label")
    promotions_create.add_argument(
        "--status",
        help="Initial promotion status (defaults to pending)",
    )
    promotions_create.add_argument(
        "--note",
        dest="notes",
        action="append",
        default=None,
        help="Attach a review note (repeat for multiple)",
    )
    promotions_create.set_defaults(handler=_marketplace_promotions_create)
    _add_common_arguments(promotions_create)

    promotions_transition = promotions_sub.add_parser(
        "transition", help="Transition a promotion gate"
    )
    promotions_transition.add_argument(
        "provider_id", help="Provider identifier (UUID)"
    )
    promotions_transition.add_argument(
        "promotion_id", help="Promotion identifier (UUID)"
    )
    promotions_transition.add_argument(
        "--status", required=True, help="New promotion status (e.g. approved)"
    )
    promotions_transition.add_argument(
        "--closed-at",
        dest="closed_at",
        help="Closure timestamp (RFC3339) for completed promotions",
    )
    promotions_transition.add_argument(
        "--note",
        dest="notes",
        action="append",
        default=None,
        help="Replace promotion notes (repeat for multiple entries)",
    )
    promotions_transition.set_defaults(handler=_marketplace_promotions_transition)
    _add_common_arguments(promotions_transition)


def install_policy(subparsers: _SubParsersAction[ArgumentParser]) -> None:
    parser = subparsers.add_parser("policy", help="Runtime policy insights")
    policy_sub = parser.add_subparsers(dest="policy_cmd", required=True)

    intelligence_parser = policy_sub.add_parser(
        "intelligence", help="Show capability intelligence scores for a server"
    )
    intelligence_parser.add_argument("server_id", type=int)
    intelligence_parser.set_defaults(handler=_policy_intelligence_scores)
    _add_common_arguments(intelligence_parser)

    vm_parser = policy_sub.add_parser(
        "vm",
        help="Inspect virtual machine attestation posture for a server",
    )
    vm_parser.add_argument("server_id", type=int)
    vm_parser.set_defaults(handler=_policy_vm_runtime)
    _add_common_arguments(vm_parser)

    watch_parser = policy_sub.add_parser(
        "watch",
        help="Stream runtime policy and attestation updates in real time",
    )
    watch_parser.add_argument(
        "--server-id",
        dest="server_id",
        type=int,
        help="Restrict the stream to a specific server identifier",
    )
    watch_parser.set_defaults(handler=_policy_watch)
    _add_common_arguments(watch_parser)


def install_trust(subparsers: _SubParsersAction[ArgumentParser]) -> None:
    parser = subparsers.add_parser("trust", help="Trust registry control plane")
    trust_sub = parser.add_subparsers(dest="trust_cmd", required=True)

    registry_parser = trust_sub.add_parser(
        "registry", help="List runtime VM trust registry entries",
    )
    registry_parser.add_argument("--server-id", type=int, dest="server_id")
    registry_parser.add_argument("--lifecycle", dest="lifecycle_state")
    registry_parser.add_argument("--status", dest="attestation_status")
    stale_group = registry_parser.add_mutually_exclusive_group()
    stale_group.add_argument("--stale", dest="stale", action="store_true")
    stale_group.add_argument("--fresh", dest="stale", action="store_false")
    registry_parser.set_defaults(stale=None)
    registry_parser.set_defaults(handler=_trust_registry)
    _add_common_arguments(registry_parser)

    get_parser = trust_sub.add_parser(
        "get", help="Fetch trust registry state for a VM instance",
    )
    get_parser.add_argument("vm_instance_id", type=int)
    get_parser.set_defaults(handler=_trust_get)
    _add_common_arguments(get_parser)

    history_parser = trust_sub.add_parser(
        "history", help="Show lifecycle history for a VM instance",
    )
    history_parser.add_argument("vm_instance_id", type=int)
    history_parser.add_argument("--limit", type=int, default=25)
    history_parser.set_defaults(handler=_trust_history)
    _add_common_arguments(history_parser)

    transition_parser = trust_sub.add_parser(
        "transition", help="Apply a registry transition for a VM instance",
    )
    transition_parser.add_argument("vm_instance_id", type=int)
    transition_parser.add_argument("--status", dest="attestation_status", required=True)
    transition_parser.add_argument("--lifecycle", dest="lifecycle_state", required=True)
    transition_parser.add_argument("--remediation-state", dest="remediation_state")
    transition_parser.add_argument(
        "--remediation-attempts", dest="remediation_attempts", type=int
    )
    transition_parser.add_argument("--freshness-deadline", dest="freshness_deadline")
    transition_parser.add_argument("--provenance-ref", dest="provenance_ref")
    transition_parser.add_argument("--provenance", dest="provenance")
    transition_parser.add_argument("--metadata", dest="metadata")
    transition_parser.add_argument("--reason", dest="transition_reason")
    transition_parser.add_argument("--expected-version", dest="expected_version", type=int)
    transition_parser.set_defaults(handler=_trust_transition)
    _add_common_arguments(transition_parser)

    watch_parser = trust_sub.add_parser(
        "watch", help="Stream live trust registry transitions",
    )
    watch_parser.add_argument("--server-id", type=int, dest="server_id")
    watch_parser.add_argument("--lifecycle", dest="lifecycle_state")
    watch_parser.add_argument("--status", dest="attestation_status")
    watch_parser.set_defaults(handler=_trust_watch)
    _add_common_arguments(watch_parser)


def install_remediation(subparsers: _SubParsersAction[ArgumentParser]) -> None:
    remediation_commands.install(subparsers, _add_common_arguments)


def install_lifecycle(subparsers: _SubParsersAction[ArgumentParser]) -> None:
    lifecycle_commands.install(subparsers, _add_common_arguments)


def install_keys(subparsers: _SubParsersAction[ArgumentParser]) -> None:
    keys_commands.install(subparsers, _add_common_arguments)


def install_billing(subparsers: _SubParsersAction[ArgumentParser]) -> None:
    billing_commands.install(subparsers, _add_common_arguments)


def install_promotions(subparsers: _SubParsersAction[ArgumentParser]) -> None:
    parser = subparsers.add_parser("promotions", help="Promotion workflow commands")
    promotions_sub = parser.add_subparsers(dest="promotions_cmd", required=True)

    tracks_parser = promotions_sub.add_parser("tracks", help="Manage promotion tracks")
    tracks_sub = tracks_parser.add_subparsers(dest="tracks_cmd", required=True)

    list_tracks = tracks_sub.add_parser("list", help="List available promotion tracks")
    list_tracks.set_defaults(handler=_promotions_list_tracks)
    _add_common_arguments(list_tracks)

    history_parser = promotions_sub.add_parser("history", help="Inspect promotion history")
    history_parser.add_argument("--track-id", type=int, help="Filter by track identifier")
    history_parser.add_argument(
        "--manifest", dest="manifest_digest", help="Filter by manifest digest"
    )
    history_parser.set_defaults(handler=_promotions_history)
    _add_common_arguments(history_parser)

    schedule_parser = promotions_sub.add_parser("schedule", help="Schedule a promotion")
    schedule_parser.add_argument("track_id", type=int)
    schedule_parser.add_argument("manifest_digest")
    schedule_parser.add_argument("stage")
    schedule_parser.add_argument(
        "--artifact-run-id", type=int, dest="artifact_run_id", help="Linked artifact run"
    )
    schedule_parser.add_argument(
        "--note", dest="notes", action="append", default=[], help="Add a scheduling note"
    )
    schedule_parser.set_defaults(handler=_promotions_schedule)
    _add_common_arguments(schedule_parser)

    approve_parser = promotions_sub.add_parser("approve", help="Approve a scheduled promotion")
    approve_parser.add_argument("promotion_id", type=int)
    approve_parser.add_argument("--note", help="Optional approval note")
    approve_parser.set_defaults(handler=_promotions_approve)
    _add_common_arguments(approve_parser)


def install_governance(subparsers: _SubParsersAction[ArgumentParser]) -> None:
    parser = subparsers.add_parser("governance", help="Governance engine commands")
    governance_sub = parser.add_subparsers(dest="governance_cmd", required=True)

    workflows_parser = governance_sub.add_parser("workflows", help="Manage governance workflows")
    workflows_sub = workflows_parser.add_subparsers(dest="workflows_cmd", required=True)

    list_workflows = workflows_sub.add_parser("list", help="List workflows")
    list_workflows.set_defaults(handler=_governance_list_workflows)
    _add_common_arguments(list_workflows)

    start_workflow = workflows_sub.add_parser("start", help="Start a workflow run")
    start_workflow.add_argument("workflow_id", type=int)
    start_workflow.add_argument(
        "--manifest-digest",
        dest="manifest_digest",
        help="Manifest digest associated with the run",
    )
    start_workflow.add_argument(
        "--artifact-run-id",
        dest="artifact_run_id",
        type=int,
        help="Artifact run to seed the workflow",
    )
    start_workflow.add_argument(
        "--context",
        dest="context",
        help="JSON context payload for the workflow",
    )
    start_workflow.set_defaults(handler=_governance_start_workflow)
    _add_common_arguments(start_workflow)

    runs_parser = governance_sub.add_parser("runs", help="Inspect governance runs")
    runs_sub = runs_parser.add_subparsers(dest="runs_cmd", required=True)

    get_run = runs_sub.add_parser("get", help="Fetch a run detail")
    get_run.add_argument("run_id", type=int)
    get_run.set_defaults(handler=_governance_get_run)
    _add_common_arguments(get_run)


def install_evaluations(subparsers: _SubParsersAction[ArgumentParser]) -> None:
    evaluations_commands.install(subparsers, _add_common_arguments)


def install_vector_dbs(subparsers: _SubParsersAction[ArgumentParser]) -> None:
    vector_db_commands.install(subparsers, _add_common_arguments)


def install_scaffold(subparsers: _SubParsersAction[ArgumentParser]) -> None:
    parser = subparsers.add_parser("scaffold", help="Agent scaffolding helpers")
    scaffold_sub = parser.add_subparsers(dest="scaffold_cmd", required=True)

    fetch_parser = scaffold_sub.add_parser("fetch-config", help="Fetch client configuration")
    fetch_parser.add_argument("server_id")
    fetch_parser.set_defaults(handler=_scaffold_fetch)
    _add_common_arguments(fetch_parser)

    python_parser = scaffold_sub.add_parser("gen-python", help="Generate Python SDK")
    python_parser.add_argument("server_id")
    python_parser.add_argument("--output", default="mcp_client.py")
    python_parser.set_defaults(handler=_scaffold_python)
    _add_common_arguments(python_parser)

    ts_parser = scaffold_sub.add_parser("gen-ts", help="Generate TypeScript SDK")
    ts_parser.add_argument("server_id")
    ts_parser.add_argument("--output", default="mcp_client.ts")
    ts_parser.set_defaults(handler=_scaffold_ts)
    _add_common_arguments(ts_parser)

    create_parser = scaffold_sub.add_parser("create", help="Create a FastAPI agent scaffold")
    create_parser.add_argument("name")
    create_parser.add_argument("--server-id", required=True)
    create_parser.add_argument(
        "--template",
        default="python-fastapi",
        choices=["python-fastapi"],
        help="Scaffold template to use",
    )
    create_parser.set_defaults(handler=_scaffold_create)
    _add_common_arguments(create_parser)


# --- Command implementations -------------------------------------------------


def _marketplace_list(client: APIClient, as_json: bool, _: Dict[str, object]) -> None:
    data = client.get("/api/marketplace")
    if as_json:
        print(dumps_json(data))
        return
    columns = ["id", "name", "tier", "status"]
    rows = []
    for item in data:
        rows.append({
            "id": item.get("id"),
            "name": item.get("name"),
            "tier": item.get("tier"),
            "status": item.get("status", item.get("state", "unknown")),
        })
    print(render_table(rows, columns))


def _marketplace_submissions_list(
    client: APIClient, as_json: bool, args: Dict[str, object]
) -> None:
    provider_id = args["provider_id"]
    path = f"/api/marketplace/providers/{provider_id}/submissions"
    payload = client.get(path)
    if as_json:
        print(dumps_json(payload))
        return

    rows = []
    for entry in payload:
        submission = entry.get("submission", {})
        evaluations: Iterable[Dict[str, Any]] = entry.get("evaluations", [])
        latest_eval = None
        if evaluations:
            latest_eval = evaluations[0].get("evaluation", {})
        rows.append(
            {
                "submission": submission.get("id"),
                "tier": submission.get("tier"),
                "status": submission.get("status"),
                "posture": _summarize_posture(submission),
                "latest_eval": _summarize_evaluation(latest_eval),
                "updated_at": submission.get("updated_at"),
            }
        )

    columns = ["submission", "tier", "status", "posture", "latest_eval", "updated_at"]
    print(render_table(rows, columns))


def _marketplace_submissions_create(
    client: APIClient, as_json: bool, args: Dict[str, object]
) -> None:
    provider_id = args["provider_id"]
    metadata = _loads_optional_json(args.get("metadata"))
    release_notes = _resolve_release_notes(
        args.get("release_notes"), args.get("release_notes_file")
    )
    payload: Dict[str, Any] = {
        "tier": args["tier"],
        "manifest_uri": args["manifest_uri"],
    }
    if args.get("artifact_digest"):
        payload["artifact_digest"] = args["artifact_digest"]
    if release_notes:
        payload["release_notes"] = release_notes
    if metadata is not None:
        payload["metadata"] = metadata

    submission = client.post(
        f"/api/marketplace/providers/{provider_id}/submissions",
        json_body=payload,
    )
    if as_json:
        print(dumps_json(submission))
        return

    message = (
        f"Created submission {submission.get('id')} "
        f"status={submission.get('status')} tier={submission.get('tier')}"
    )
    print(message)


def _marketplace_evaluations_list(
    client: APIClient, as_json: bool, args: Dict[str, object]
) -> None:
    provider_id = args["provider_id"]
    submission_filter = args.get("submission_id")
    payload = client.get(f"/api/marketplace/providers/{provider_id}/submissions")
    evaluations_rows = []
    for entry in payload:
        submission = entry.get("submission", {})
        submission_id = submission.get("id")
        if submission_filter and submission_filter != submission_id:
            continue
        for evaluation_entry in entry.get("evaluations", []):
            evaluation = evaluation_entry.get("evaluation", {})
            promotions = evaluation_entry.get("promotions", [])
            evaluations_rows.append(
                {
                    "submission": submission_id,
                    "evaluation": evaluation.get("id"),
                    "type": evaluation.get("evaluation_type"),
                    "status": evaluation.get("status"),
                    "posture": _summarize_posture(evaluation),
                    "promotions": ",".join(
                        filter(None, (promo.get("status") for promo in promotions))
                    ),
                    "updated_at": evaluation.get("updated_at"),
                }
            )

    if as_json:
        print(dumps_json(evaluations_rows))
        return

    columns = [
        "submission",
        "evaluation",
        "type",
        "status",
        "posture",
        "promotions",
        "updated_at",
    ]
    print(render_table(evaluations_rows, columns))


def _marketplace_evaluations_start(
    client: APIClient, as_json: bool, args: Dict[str, object]
) -> None:
    provider_id = args["provider_id"]
    submission_id = args["submission_id"]
    payload: Dict[str, Any] = {
        "evaluation_type": args["evaluation_type"],
    }
    if args.get("status"):
        payload["status"] = args["status"]
    if args.get("evaluator_ref"):
        payload["evaluator_ref"] = args["evaluator_ref"]
    if args.get("result"):
        payload["result"] = _loads_json(args["result"])

    evaluation = client.post(
        f"/api/marketplace/providers/{provider_id}/submissions/{submission_id}/evaluations",
        json_body=payload,
    )
    if as_json:
        print(dumps_json(evaluation))
        return
    print(
        f"Started evaluation {evaluation.get('id')} "
        f"status={evaluation.get('status')} type={evaluation.get('evaluation_type')}"
    )


def _marketplace_evaluations_transition(
    client: APIClient, as_json: bool, args: Dict[str, object]
) -> None:
    provider_id = args["provider_id"]
    evaluation_id = args["evaluation_id"]
    payload: Dict[str, Any] = {"status": args["status"]}
    if args.get("completed_at"):
        payload["completed_at"] = args["completed_at"]
    if args.get("result"):
        payload["result"] = _loads_json(args["result"])

    evaluation = client.post(
        f"/api/marketplace/providers/{provider_id}/evaluations/{evaluation_id}/transition",
        json_body=payload,
    )
    if as_json:
        print(dumps_json(evaluation))
        return
    print(
        f"Transitioned evaluation {evaluation.get('id')} "
        f"status={evaluation.get('status')}"
    )


def _marketplace_promotions_create(
    client: APIClient, as_json: bool, args: Dict[str, object]
) -> None:
    provider_id = args["provider_id"]
    evaluation_id = args["evaluation_id"]
    payload: Dict[str, Any] = {"gate": args["gate"]}
    if args.get("status"):
        payload["status"] = args["status"]
    notes = _normalize_notes(args.get("notes"))
    if notes is not None:
        payload["notes"] = notes

    promotion = client.post(
        f"/api/marketplace/providers/{provider_id}/evaluations/{evaluation_id}/promotions",
        json_body=payload,
    )
    if as_json:
        print(dumps_json(promotion))
        return

    print(
        f"Created promotion {promotion.get('id')} "
        f"gate={promotion.get('gate')} status={promotion.get('status')}"
    )


def _marketplace_promotions_transition(
    client: APIClient, as_json: bool, args: Dict[str, object]
) -> None:
    provider_id = args["provider_id"]
    promotion_id = args["promotion_id"]
    payload: Dict[str, Any] = {"status": args["status"]}
    if args.get("closed_at"):
        payload["closed_at"] = args["closed_at"]
    notes = _normalize_notes(args.get("notes"))
    if notes is not None:
        payload["notes"] = notes

    promotion = client.post(
        f"/api/marketplace/providers/{provider_id}/promotions/{promotion_id}/transition",
        json_body=payload,
    )
    if as_json:
        print(dumps_json(promotion))
        return

    print(
        f"Transitioned promotion {promotion.get('id')} "
        f"status={promotion.get('status')}"
    )


def _marketplace_watch(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    provider_id = args["provider_id"]
    max_events = int(args.get("max_events") or 0)
    stream_path = f"/api/marketplace/providers/{provider_id}/events/stream"
    count = 0
    try:
        for raw_line in client.stream_sse(stream_path):
            if as_json:
                print(raw_line)
            else:
                print(_summarize_marketplace_event(raw_line))
            count += 1
            if max_events and count >= max_events:
                break
    except KeyboardInterrupt:  # pragma: no cover - user interruption
        pass


def _summarize_marketplace_event(raw_line: str) -> str:
    try:
        event = json.loads(raw_line)
    except json.JSONDecodeError:
        return raw_line

    event_type = event.get("event_type", "event")
    occurred_at = event.get("occurred_at", "?")
    submission = event.get("submission_id")
    evaluation = event.get("evaluation_id")
    promotion = event.get("promotion_id")
    payload = event.get("payload", {}) if isinstance(event.get("payload"), dict) else {}
    status = payload.get("status") or payload.get("state")
    pieces = [f"[{occurred_at}]", event_type]
    if submission:
        pieces.append(f"submission={submission}")
    if evaluation:
        pieces.append(f"evaluation={evaluation}")
    if promotion:
        pieces.append(f"promotion={promotion}")
    if status:
        pieces.append(f"status={status}")
    actor = event.get("actor_ref")
    if actor:
        pieces.append(f"actor={actor}")
    notes = payload.get("notes")
    if isinstance(notes, list) and notes:
        pieces.append(f"notes={len(notes)}")
    return " ".join(pieces)


def _summarize_posture(record: Dict[str, Any]) -> str:
    if record.get("posture_vetoed"):
        return "vetoed"
    notes = record.get("posture_notes") or []
    if notes:
        return f"notes:{len(notes)}"
    return "clear"


def _summarize_evaluation(record: Dict[str, Any] | None) -> str:
    if not record:
        return "—"
    status = record.get("status", "unknown")
    eval_type = record.get("evaluation_type", "?")
    return f"{eval_type}:{status}"


def _loads_optional_json(value: object) -> Dict[str, Any] | list[Any] | None:
    if not value:
        return None
    return _loads_json(value)


def _loads_json(value: object) -> Any:
    if value is None:
        raise ValueError("JSON payload cannot be None")
    if isinstance(value, (dict, list)):
        return value
    if isinstance(value, str):
        try:
            return json.loads(value)
        except json.JSONDecodeError as exc:  # pragma: no cover - defensive path
            raise ValueError(f"Invalid JSON payload: {exc}") from exc
    raise ValueError("Unsupported JSON payload type")


def _resolve_release_notes(text: object, file_path: object) -> str | None:
    if file_path:
        path = Path(str(file_path))
        content = path.read_text(encoding="utf-8").strip()
        if not content:
            raise ValueError("Release notes file is empty")
        return content
    if text:
        return str(text)
    return None


def _normalize_notes(value: object) -> list[str] | None:
    if value is None:
        return None
    if isinstance(value, list):
        notes = [str(entry).strip() for entry in value if str(entry).strip()]
        return notes if notes else None
    if isinstance(value, str):
        cleaned = value.strip()
        return [cleaned] if cleaned else None
    raise ValueError(
        "Notes must be provided as repeated --note arguments or string values"
    )


def _promotions_list_tracks(client: APIClient, as_json: bool, _: Dict[str, object]) -> None:
    tracks = client.get("/api/promotions/tracks")
    if as_json:
        print(dumps_json(tracks))
        return
    columns = ["id", "name", "tier", "stages"]
    records = [{
        "id": track.get("id"),
        "name": track.get("name"),
        "tier": track.get("tier"),
        "stages": track.get("stages", []),
    } for track in tracks]
    print(render_table(records, columns))


def _promotions_history(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    params = {key: value for key, value in args.items() if key in {"manifest_digest", "track_id"} and value}
    history = client.get("/api/promotions/history", params=params if params else None)
    if as_json:
        print(dumps_json(history))
        return
    columns = [
        "id",
        "track_name",
        "stage",
        "status",
        "posture",
        "manifest_digest",
        "updated_at",
    ]
    records = []
    for item in history:
        verdict = item.get("posture_verdict") if isinstance(item, dict) else None
        posture = _summarize_promotion_posture(verdict)
        records.append(
            {
                "id": item.get("id"),
                "track_name": item.get("track_name"),
                "stage": item.get("stage"),
                "status": item.get("status"),
                "posture": posture,
                "manifest_digest": item.get("manifest_digest"),
                "updated_at": item.get("updated_at"),
            }
        )
    print(render_table(records, columns))


def _promotions_schedule(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    payload = {
        "track_id": args["track_id"],
        "manifest_digest": args["manifest_digest"],
        "stage": args["stage"],
        "artifact_run_id": args.get("artifact_run_id"),
        "notes": args.get("notes") or [],
    }
    try:
        result = client.post("/api/promotions/schedule", json_body=payload)
    except APIError as exc:
        if as_json:
            print(dumps_json(exc.payload or {"error": str(exc)}))
            return
        print(f"Promotion scheduling failed: {exc}")
        _render_promotion_veto(exc.payload)
        return

    if as_json:
        print(dumps_json(result))
        return

    posture = _summarize_promotion_posture(result.get("posture_verdict"))
    columns = ["id", "track_name", "stage", "status", "posture"]
    print(
        render_table(
            [
                {
                    "id": result.get("id"),
                    "track_name": result.get("track_name"),
                    "stage": result.get("stage"),
                    "status": result.get("status"),
                    "posture": posture,
                }
            ],
            columns,
        )
    )


def _promotions_approve(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    payload = {"note": args.get("note")}
    result = client.post(
        f"/api/promotions/{args['promotion_id']}/approve",
        json_body=payload if payload.get("note") else None,
    )
    if as_json:
        print(dumps_json(result))
    else:
        columns = ["id", "stage", "status", "approved_at"]
        print(render_table([result], columns))


def _governance_list_workflows(client: APIClient, as_json: bool, _: Dict[str, object]) -> None:
    workflows = client.get("/api/governance/workflows")
    if as_json:
        print(dumps_json(workflows))
        return
    columns = ["id", "name", "description"]
    rows = [{
        "id": item.get("id"),
        "name": item.get("name"),
        "description": item.get("description", ""),
    } for item in workflows]
    print(render_table(rows, columns))


def _governance_start_workflow(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    payload = {
        "manifest_digest": args.get("manifest_digest"),
        "artifact_run_id": args.get("artifact_run_id"),
    }
    if args.get("context"):
        try:
            payload["context"] = json.loads(args["context"])  # type: ignore[arg-type]
        except json.JSONDecodeError as exc:  # pragma: no cover - validated in tests
            raise ValueError(f"Invalid JSON for --context: {exc}") from exc
    payload = {key: value for key, value in payload.items() if value is not None}
    result = client.post(
        f"/api/governance/workflows/{args['workflow_id']}/runs",
        json_body=payload if payload else None,
    )
    if as_json:
        print(dumps_json(result))
    else:
        columns = ["run_id", "workflow_id", "status", "created_at"]
        print(render_table([{
            "run_id": result.get("id"),
            "workflow_id": result.get("workflow_id"),
            "status": result.get("status"),
            "created_at": result.get("created_at"),
        }], columns))


def _governance_get_run(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    detail = client.get(f"/api/governance/runs/{args['run_id']}")
    if as_json:
        print(dumps_json(detail))
    else:
        columns = ["id", "workflow_id", "status", "updated_at"]
        print(render_table([detail], columns))


def _summarize_promotion_posture(verdict: Any) -> str:
    if not isinstance(verdict, dict):
        return "unknown"
    allowed = verdict.get("allowed")
    reasons = verdict.get("reasons") if isinstance(verdict.get("reasons"), list) else []
    if allowed is True and not reasons:
        return "allowed"
    if allowed is True:
        return "allowed with notes"
    if allowed is False and reasons:
        return "blocked: " + "; ".join(str(reason) for reason in reasons)
    if allowed is False:
        return "blocked"
    return "unknown"


def _render_promotion_veto(payload: Any) -> None:
    if not isinstance(payload, dict):
        return
    reasons = payload.get("reasons") if isinstance(payload.get("reasons"), list) else []
    notes = payload.get("notes") if isinstance(payload.get("notes"), list) else []
    metadata = payload.get("metadata") if isinstance(payload.get("metadata"), dict) else {}

    if reasons:
        print("\nVeto reasons:")
        print(render_table([{"reason": str(reason)} for reason in reasons], ["reason"]))
    if notes:
        print("\nPosture notes:")
        print(render_table([{"note": str(note)} for note in notes], ["note"]))

    trust = metadata.get("signals", {}).get("trust") if isinstance(metadata.get("signals"), dict) else None
    remediation = metadata.get("signals", {}).get("remediation") if isinstance(metadata.get("signals"), dict) else None
    intelligence = metadata.get("signals", {}).get("intelligence") if isinstance(metadata.get("signals"), dict) else []

    if isinstance(trust, dict) and trust:
        print("\nTrust posture:")
        trust_rows = []
        for field in ("lifecycle_state", "attestation_status", "remediation_state"):
            value = trust.get(field)
            if value is not None:
                trust_rows.append({"field": field, "value": value})
        attempts = trust.get("remediation_attempts")
        if attempts is not None:
            trust_rows.append({"field": "remediation_attempts", "value": attempts})
        if trust_rows:
            print(render_table(trust_rows, ["field", "value"]))

    if isinstance(remediation, dict) and remediation:
        print("\nRemediation posture:")
        rem_rows = []
        status = remediation.get("status")
        if status is not None:
            rem_rows.append({"field": "status", "value": status})
        failure = remediation.get("failure_reason")
        if failure:
            rem_rows.append({"field": "failure_reason", "value": failure})
        if rem_rows:
            print(render_table(rem_rows, ["field", "value"]))

    if isinstance(intelligence, list) and intelligence:
        print("\nIntelligence signals:")
        intel_rows = []
        for entry in intelligence:
            if not isinstance(entry, dict):
                continue
            intel_rows.append(
                {
                    "capability": entry.get("capability"),
                    "status": entry.get("status"),
                    "score": entry.get("score"),
                    "confidence": entry.get("confidence"),
                }
            )
        if intel_rows:
            print(render_table(intel_rows, ["capability", "status", "score", "confidence"]))
def _scaffold_fetch(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    data = client.get(f"/api/servers/{args['server_id']}/client-config")
    if as_json:
        print(dumps_json(data))
    else:
        print(dumps_json(data))


def _scaffold_python(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    from ..scaffold import generate_python_sdk

    cfg = client.get(f"/api/servers/{args['server_id']}/client-config")
    code = generate_python_sdk(cfg)
    with open(args["output"], "w", encoding="utf-8") as handle:
        handle.write(code)
    if not as_json:
        print(f"Python SDK written to {args['output']}")


def _scaffold_ts(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    from ..scaffold import generate_ts_sdk

    cfg = client.get(f"/api/servers/{args['server_id']}/client-config")
    code = generate_ts_sdk(cfg)
    with open(args["output"], "w", encoding="utf-8") as handle:
        handle.write(code)
    if not as_json:
        print(f"TypeScript SDK written to {args['output']}")


def _scaffold_create(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    if args.get("template") != "python-fastapi":
        raise ValueError("Only the python-fastapi template is supported")

    from pathlib import Path

    from ..scaffold import write_fastapi_project

    cfg = client.get(f"/api/servers/{args['server_id']}/client-config")
    project_dir = Path(args["name"])
    write_fastapi_project(project_dir, cfg)
    if not as_json:
        print(f"Scaffold created in {project_dir}")

def _policy_intelligence_scores(
    client: APIClient, as_json: bool, args: Dict[str, object]
) -> None:
    server_id = args["server_id"]
    scores = client.get(f"/api/intelligence/servers/{server_id}/scores")
    if as_json:
        print(dumps_json(scores))
        return

    columns = [
        "capability",
        "score",
        "status",
        "backend",
        "tier",
        "observed_at",
        "notes",
    ]
    rows = []
    for entry in scores:
        rows.append({
            "capability": entry.get("capability"),
            "score": f"{entry.get('score', 0):.1f}",
            "status": entry.get("status"),
            "backend": entry.get("backend") or "-",
            "tier": entry.get("tier") or "-",
            "observed_at": entry.get("last_observed_at"),
            "notes": "; ".join(entry.get("notes", [])[:3]),
        })
    print(render_table(rows, columns))


def _policy_vm_runtime(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    server_id = args["server_id"]
    summary = client.get(f"/api/servers/{server_id}/vm")
    if as_json:
        print(dumps_json(summary))
        return

    instances = summary.get("instances", [])
    if not instances:
        print("No VM instances recorded for this server")
        return

    columns = ["instance", "status", "tier", "updated", "active"]
    active = summary.get("active_instance_id")
    rows = []
    for entry in instances:
        instance_id = entry.get("instance_id")
        rows.append(
            {
                "instance": instance_id,
                "status": entry.get("attestation_status"),
                "tier": entry.get("isolation_tier") or "-",
                "updated": entry.get("updated_at"),
                "active": "yes" if active and instance_id == active else "",
            }
        )

    print(render_table(rows, columns))
    latest = summary.get("latest_status", "unknown")
    updated = summary.get("last_updated_at", "unknown")
    print(f"Latest posture: {latest} (updated {updated})")
    if active:
        print(f"Active instance: {active}")


def _policy_watch(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    params: Dict[str, Any] = {}
    server_id = args.get("server_id")
    if server_id is not None:
        params["server_id"] = server_id

    state: Dict[int, Dict[str, Any]] = {}
    use_color = sys.stdout.isatty() and not as_json

    try:
        for payload in client.stream_sse("/api/policy/stream", params=params or None):
            if not payload:
                continue
            try:
                event = json.loads(payload)
            except json.JSONDecodeError:
                if as_json:
                    print(payload)
                continue

            if as_json:
                print(dumps_json(event))
                continue

            rendered = _render_policy_event(event, state, use_color)
            if rendered:
                print(rendered)
    except KeyboardInterrupt:  # pragma: no cover - user initiated
        pass


def _render_policy_event(
    event: Dict[str, Any],
    state: Dict[int, Dict[str, Any]],
    use_color: bool,
) -> str | None:
    server_id = event.get("server_id")
    if not isinstance(server_id, int):
        return None

    timestamp = event.get("timestamp", "")
    event_type = str(event.get("type", "unknown"))
    summary = state.setdefault(server_id, {})
    header = f"[{timestamp}] server {server_id} {event_type.upper()}"

    changes: list[str] = []

    backend = event.get("backend")
    if isinstance(backend, str):
        previous = summary.get("backend")
        summary["backend"] = backend
        if previous is None:
            changes.append(f"backend {backend}")
        elif previous != backend:
            changes.append(f"backend {previous} -> {backend}")

    candidate = event.get("candidate_backend")
    if isinstance(candidate, str):
        summary["candidate_backend"] = candidate

    fallback_backend = event.get("fallback_backend")
    if isinstance(fallback_backend, str):
        summary["fallback_backend"] = fallback_backend
        changes.append(f"fallback -> {fallback_backend}")

    instance_id = event.get("instance_id")
    if isinstance(instance_id, str):
        summary["active_instance"] = instance_id

    att_status = event.get("attestation_status")
    if isinstance(att_status, str):
        previous = summary.get("attestation_status")
        summary["attestation_status"] = att_status
        current = _colorize_status(att_status, use_color)
        if previous is None:
            changes.append(f"attestation {current}")
        elif previous != att_status:
            changes.append(
                f"attestation {current} (was {_colorize_status(str(previous), use_color)})"
            )

    trust_event = event.get("trust_event")
    if isinstance(trust_event, dict):
        summary["trust_event_id"] = trust_event.get("id")
        reason = trust_event.get("transition_reason") or "posture"
        triggered = trust_event.get("triggered_at")
        descriptor = f"trust {reason}"
        if triggered:
            descriptor += f" @ {triggered}"
        changes.append(descriptor)

    trust_event_id = event.get("trust_event_id")
    if trust_event_id is not None:
        summary["trust_event_id"] = trust_event_id

    lifecycle_state = event.get("trust_lifecycle_state")
    if isinstance(lifecycle_state, str):
        previous = summary.get("trust_lifecycle_state")
        summary["trust_lifecycle_state"] = lifecycle_state
        if previous is None:
            changes.append(f"trust lifecycle {lifecycle_state}")
        elif previous != lifecycle_state:
            changes.append(f"trust lifecycle {previous} -> {lifecycle_state}")

    prev_lifecycle = event.get("trust_previous_lifecycle_state")
    if isinstance(prev_lifecycle, str):
        summary["trust_previous_lifecycle_state"] = prev_lifecycle

    remediation_attempts = event.get("trust_remediation_attempts")
    if isinstance(remediation_attempts, int):
        previous = summary.get("trust_remediation_attempts")
        summary["trust_remediation_attempts"] = remediation_attempts
        if previous is None or previous != remediation_attempts:
            changes.append(f"trust remediation {remediation_attempts}")

    freshness_deadline = event.get("freshness_expires_at") or event.get(
        "trust_freshness_deadline"
    )
    if isinstance(freshness_deadline, str):
        summary["trust_freshness_deadline"] = freshness_deadline

    provenance_ref = event.get("trust_provenance_ref")
    if isinstance(provenance_ref, str):
        summary["trust_provenance_ref"] = provenance_ref

    provenance = event.get("trust_provenance")
    if provenance is not None:
        summary["trust_provenance"] = provenance

    stale_flag = event.get("stale")
    if isinstance(stale_flag, bool):
        previous = summary.get("stale")
        summary["stale"] = stale_flag
        if previous is None or previous != stale_flag:
            label = "stale" if stale_flag else "fresh"
            changes.append(f"evidence {label}")

    for field, label in ("evaluation_required", "evaluation"), ("governance_required", "governance"):
        value = event.get(field)
        if isinstance(value, bool):
            previous = summary.get(field)
            summary[field] = value
            if previous is None or previous != value:
                current = "required" if value else "clear"
                if previous is None:
                    changes.append(f"{label} {current}")
                else:
                    prev_label = "required" if previous else "clear"
                    changes.append(f"{label} {prev_label} -> {current}")

    provider_key = event.get("provider_key_posture")
    if isinstance(provider_key, dict):
        state_value = provider_key.get("state")
        if isinstance(state_value, str):
            prev_state = summary.get("provider_key_state")
            summary["provider_key_state"] = state_value
            if prev_state is None:
                changes.append(f"provider key {state_value}")
            elif prev_state != state_value:
                changes.append(f"provider key {prev_state} -> {state_value}")

        veto_flag = provider_key.get("vetoed")
        if isinstance(veto_flag, bool):
            prev_veto = summary.get("provider_key_vetoed")
            summary["provider_key_vetoed"] = veto_flag
            if prev_veto is None or prev_veto != veto_flag:
                label = "vetoed" if veto_flag else "cleared"
                changes.append(f"provider key {label}")

        rotation_due = provider_key.get("rotation_due_at")
        if isinstance(rotation_due, str):
            summary["provider_key_rotation_due_at"] = rotation_due

        notes = provider_key.get("notes")
        if isinstance(notes, list):
            summary["provider_key_notes"] = [str(entry) for entry in notes if isinstance(entry, str)]

        provider_id = provider_key.get("provider_id")
        if isinstance(provider_id, str):
            summary["provider_key_provider_id"] = provider_id

    instance_id = event.get("instance_id")
    if isinstance(instance_id, str):
        previous = summary.get("instance_id")
        summary["instance_id"] = instance_id
        if previous is None:
            changes.append(f"instance {instance_id}")
        elif previous != instance_id:
            changes.append(f"instance {previous} -> {instance_id}")

    signal_notes = _filter_signal_notes(event.get("notes"))
    if not changes and not signal_notes:
        return None

    parts: list[str] = []
    if changes:
        parts.append("; ".join(changes))
    active_instance = summary.get("active_instance")
    if isinstance(active_instance, str):
        parts.append(f"Active instance: {active_instance}")
    latest_posture = summary.get("attestation_status")
    if isinstance(latest_posture, str):
        parts.append(f"Latest posture: {latest_posture}")
    provider_key_state = summary.get("provider_key_state")
    if isinstance(provider_key_state, str):
        descriptor = f"Provider key: {provider_key_state}"
        if summary.get("provider_key_vetoed") is True:
            descriptor += " (vetoed)"
        parts.append(descriptor)
        rotation_due = summary.get("provider_key_rotation_due_at")
        if isinstance(rotation_due, str):
            parts.append(f"BYOK rotation due @ {rotation_due}")
    if signal_notes:
        parts.append(", ".join(signal_notes))

    return f"{header} {' | '.join(parts)}"


def _filter_signal_notes(notes: Any) -> list[str]:
    if not isinstance(notes, list):
        return []
    signals: list[str] = []
    for note in notes:
        if isinstance(note, str) and (
            note.startswith("vm:attestation")
            or note.startswith("attestation:")
            or "fallback" in note
            or note.startswith("provider-key:")
        ):
            signals.append(note)
    return signals


def _colorize_status(status: str, use_color: bool) -> str:
    normalized = status.replace("_", "-").lower()
    if not use_color:
        return normalized

    mapping = {
        "trusted": _GREEN,
        "untrusted": _RED,
        "unknown": _YELLOW,
        "pending": _YELLOW,
        "stale": _YELLOW,
    }
    color = mapping.get(normalized, _CYAN)
    return f"{color}{normalized}{_RESET}"


def _trust_registry(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    params: Dict[str, Any] = {}
    server_id = args.get("server_id")
    if server_id is not None:
        params["server_id"] = server_id
    lifecycle = args.get("lifecycle_state")
    if lifecycle:
        params["lifecycle_state"] = lifecycle
    status = args.get("attestation_status")
    if status:
        params["attestation_status"] = status
    stale = args.get("stale")
    if stale is not None:
        params["stale"] = "true" if stale else "false"

    entries = client.get("/api/trust/registry", params=params or None)
    if as_json:
        print(dumps_json(entries))
        return

    if not entries:
        print("No trust registry entries found")
        return

    columns = [
        "server",
        "instance",
        "status",
        "lifecycle",
        "remediation",
        "attempts",
        "stale",
        "updated",
    ]
    rows: list[Dict[str, Any]] = []
    for entry in entries:
        server = entry.get("server_name") or "unknown"
        server_id_val = entry.get("server_id")
        rows.append(
            {
                "server": f"{server} ({server_id_val})",
                "instance": entry.get("instance_id"),
                "status": entry.get("attestation_status"),
                "lifecycle": entry.get("lifecycle_state"),
                "remediation": entry.get("remediation_state") or "-",
                "attempts": entry.get("remediation_attempts"),
                "stale": "yes" if entry.get("stale") else "",
                "updated": entry.get("updated_at"),
            }
        )

    print(render_table(rows, columns))


def _trust_get(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    instance_id = args["vm_instance_id"]
    state = client.get(f"/api/trust/registry/{instance_id}")
    if as_json:
        print(dumps_json(state))
        return

    server_name = state.get("server_name") or "unknown"
    print(f"Server: {server_name} ({state.get('server_id')})")
    print(f"VM Instance: {state.get('instance_id')} ({state.get('vm_instance_id')})")
    print(f"Attestation: {state.get('attestation_status')}")
    print(f"Lifecycle: {state.get('lifecycle_state')}")
    remediation_state = state.get("remediation_state") or "-"
    print(f"Remediation: {remediation_state} (attempts {state.get('remediation_attempts')})")
    freshness = state.get("freshness_deadline") or "unset"
    print(f"Freshness deadline: {freshness}")
    print(f"Provenance ref: {state.get('provenance_ref') or '-'}")
    print(f"Version: {state.get('version')} (updated {state.get('updated_at')})")


def _trust_history(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    instance_id = args["vm_instance_id"]
    params: Dict[str, Any] = {}
    limit = args.get("limit")
    if limit:
        params["limit"] = limit
    history = client.get(
        f"/api/trust/registry/{instance_id}/history", params=params or None
    )
    if as_json:
        print(dumps_json(history))
        return

    print(
        f"Server {history.get('server_name')} ({history.get('server_id')})"
        f" instance {history.get('instance_id')}"
    )
    events = history.get("events", [])
    if not events:
        print("No trust transitions recorded")
        return

    columns = ["triggered", "status", "lifecycle", "remediation", "attempts", "reason"]
    rows = []
    for event in events:
        rows.append(
            {
                "triggered": event.get("triggered_at"),
                "status": event.get("current_status"),
                "lifecycle": event.get("current_lifecycle_state"),
                "remediation": event.get("remediation_state") or "-",
                "attempts": event.get("remediation_attempts"),
                "reason": event.get("transition_reason") or "-",
            }
        )

    print(render_table(rows, columns))


def _trust_transition(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    instance_id = args["vm_instance_id"]
    payload: Dict[str, Any] = {
        "attestation_status": args["attestation_status"],
        "lifecycle_state": args["lifecycle_state"],
    }
    if args.get("remediation_state"):
        payload["remediation_state"] = args["remediation_state"]
    if args.get("remediation_attempts") is not None:
        payload["remediation_attempts"] = args["remediation_attempts"]
    if args.get("freshness_deadline"):
        payload["freshness_deadline"] = args["freshness_deadline"]
    if args.get("provenance_ref"):
        payload["provenance_ref"] = args["provenance_ref"]
    if args.get("transition_reason"):
        payload["transition_reason"] = args["transition_reason"]
    if args.get("expected_version") is not None:
        payload["expected_version"] = args["expected_version"]

    for field in ("provenance", "metadata"):
        value = args.get(field)
        if value:
            try:
                payload[field] = json.loads(value)
            except json.JSONDecodeError as exc:
                raise ValueError(f"Invalid JSON for --{field.replace('_', '-')}: {exc}")

    result = client.post(
        f"/api/trust/registry/{instance_id}/transition", json_body=payload
    )
    if as_json:
        print(dumps_json(result))
        return

    print("Transition applied")
    _trust_get(client, False, {"vm_instance_id": instance_id, "json": False})


def _trust_watch(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    params: Dict[str, Any] = {}
    for key in ("server_id", "lifecycle_state", "attestation_status"):
        value = args.get(key)
        if value:
            params[key] = value

    try:
        for payload in client.stream_sse(
            "/api/trust/registry/stream", params=params or None
        ):
            if not payload:
                continue
            try:
                event = json.loads(payload)
            except json.JSONDecodeError:
                if as_json:
                    print(payload)
                continue

            if as_json:
                print(dumps_json(event))
                continue

            rendered = _render_trust_event(event)
            if rendered:
                print(rendered)
    except KeyboardInterrupt:  # pragma: no cover - user initiated
        pass


def _render_trust_event(event: Dict[str, Any]) -> str:
    server_id = event.get("server_id")
    vm_instance_id = event.get("vm_instance_id")
    if not isinstance(server_id, int) or not isinstance(vm_instance_id, int):
        return ""

    triggered = event.get("triggered_at") or "unknown"
    server_name = (event.get("server_name") or "").strip()
    status = event.get("attestation_status") or "unknown"
    previous_status = event.get("previous_attestation_status") or "-"
    lifecycle = event.get("lifecycle_state") or "-"
    previous_lifecycle = event.get("previous_lifecycle_state") or "-"
    remediation_state = event.get("remediation_state") or "-"
    attempts = event.get("remediation_attempts")
    attempts_text = attempts if isinstance(attempts, int) and attempts >= 0 else "-"
    freshness_deadline = event.get("freshness_deadline")
    stale_flag = bool(event.get("stale"))
    freshness_state = "stale" if stale_flag else "fresh"
    if freshness_deadline:
        freshness_state = f"{freshness_state} (deadline {freshness_deadline})"
    reason = event.get("transition_reason") or "-"
    provenance_ref = event.get("provenance_ref") or "-"
    version = event.get("version")
    version_text = f"v{version}" if isinstance(version, int) else ""

    header = f"[{triggered}] server {server_id}"
    if server_name:
        header += f" ({server_name})"
    header += f" vm {vm_instance_id}"

    segments = [
        header,
        f"status {previous_status} -> {status}",
        f"lifecycle {previous_lifecycle} -> {lifecycle}",
        f"remediation {remediation_state} (attempts {attempts_text})",
        f"freshness {freshness_state}",
    ]

    if provenance_ref != "-":
        segments.append(f"provenance {provenance_ref}")
    if reason != "-":
        segments.append(f"reason {reason}")
    if version_text:
        segments.append(version_text)

    return " | ".join(segment.strip() for segment in segments if segment.strip())


