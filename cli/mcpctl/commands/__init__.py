"""Command registration for the mission-control CLI."""
# key: operator-cli -> command-registry

from __future__ import annotations

import json
from argparse import ArgumentParser, _SubParsersAction
from typing import Callable, Dict

from ..client import APIClient
from ..renderers import dumps_json, render_table
from . import evaluations as evaluations_commands

CommandFn = Callable[[APIClient, bool, Dict[str, object]], None]


def _add_common_arguments(parser: ArgumentParser) -> None:
    parser.add_argument("--json", action="store_true", help="Render output as JSON")


def install_marketplace(subparsers: _SubParsersAction[ArgumentParser]) -> None:
    parser = subparsers.add_parser("marketplace", help="Marketplace operations")
    marketplace_sub = parser.add_subparsers(dest="marketplace_cmd", required=True)

    list_parser = marketplace_sub.add_parser("list", help="List marketplace offerings")
    list_parser.set_defaults(handler=_marketplace_list)
    _add_common_arguments(list_parser)


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
    columns = ["id", "track_name", "stage", "status", "manifest_digest", "updated_at"]
    records = [{
        "id": item.get("id"),
        "track_name": item.get("track_name"),
        "stage": item.get("stage"),
        "status": item.get("status"),
        "manifest_digest": item.get("manifest_digest"),
        "updated_at": item.get("updated_at"),
    } for item in history]
    print(render_table(records, columns))


def _promotions_schedule(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    payload = {
        "track_id": args["track_id"],
        "manifest_digest": args["manifest_digest"],
        "stage": args["stage"],
        "artifact_run_id": args.get("artifact_run_id"),
        "notes": args.get("notes") or [],
    }
    result = client.post("/api/promotions/schedule", json_body=payload)
    if as_json:
        print(dumps_json(result))
    else:
        columns = ["id", "track_name", "stage", "status"]
        print(render_table([result], columns))


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


