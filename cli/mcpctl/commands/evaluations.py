from __future__ import annotations

import json
from argparse import ArgumentParser, _SubParsersAction
from datetime import datetime, timezone
from typing import Any, Callable, Dict

from ..client import APIClient
from ..renderers import dumps_json, render_table

CommandFn = Callable[[APIClient, bool, Dict[str, object]], None]


def install(
    subparsers: _SubParsersAction[ArgumentParser],
    add_common_arguments: Callable[[ArgumentParser], None],
) -> None:
    parser = subparsers.add_parser("evaluations", help="Evaluation lifecycle commands")
    evaluations_sub = parser.add_subparsers(dest="evaluations_cmd", required=True)

    list_parser = evaluations_sub.add_parser("list", help="List evaluations across artifacts")
    list_parser.set_defaults(handler=_list)
    add_common_arguments(list_parser)

    retry_parser = evaluations_sub.add_parser("retry", help="Retry a certification")
    retry_parser.add_argument("evaluation_id", type=int)
    retry_parser.set_defaults(handler=_retry)
    add_common_arguments(retry_parser)

    status_parser = evaluations_sub.add_parser("status", help="Inspect certification status")
    status_parser.add_argument("evaluation_id", type=int)
    status_parser.set_defaults(handler=_status)
    add_common_arguments(status_parser)

    lineage_parser = evaluations_sub.add_parser("lineage", help="Show evidence lineage")
    lineage_parser.add_argument("evaluation_id", type=int)
    lineage_parser.set_defaults(handler=_lineage)
    add_common_arguments(lineage_parser)

    plan_parser = evaluations_sub.add_parser("plan", help="Override evidence refresh plan")
    plan_parser.add_argument("evaluation_id", type=int)
    plan_parser.add_argument(
        "--cadence-seconds",
        dest="cadence_seconds",
        type=int,
        help="Set refresh cadence in seconds",
    )
    plan_parser.add_argument(
        "--unset-cadence",
        action="store_true",
        dest="unset_cadence",
        help="Clear configured refresh cadence",
    )
    plan_parser.add_argument(
        "--next-refresh",
        dest="next_refresh",
        help="Set the next refresh timestamp (ISO-8601)",
    )
    plan_parser.add_argument(
        "--unset-next-refresh",
        action="store_true",
        dest="unset_next_refresh",
        help="Clear the scheduled next refresh timestamp",
    )
    plan_parser.add_argument("--note", dest="note", help="Replace governance notes")
    plan_parser.add_argument(
        "--unset-note",
        action="store_true",
        dest="unset_note",
        help="Clear governance notes",
    )
    plan_parser.add_argument(
        "--source",
        dest="source",
        help="Set evidence source descriptor as JSON",
    )
    plan_parser.add_argument(
        "--unset-source",
        action="store_true",
        dest="unset_source",
        help="Clear evidence source descriptor",
    )
    plan_parser.add_argument(
        "--lineage",
        dest="lineage",
        help="Set evidence lineage payload as JSON",
    )
    plan_parser.add_argument(
        "--unset-lineage",
        action="store_true",
        dest="unset_lineage",
        help="Clear evidence lineage payload",
    )
    plan_parser.set_defaults(handler=_plan)
    add_common_arguments(plan_parser)

def _normalize_iso(value: str) -> str:
    if value.endswith("Z"):
        value = value[:-1] + "+00:00"
    dt = datetime.fromisoformat(value)
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)
    return dt.astimezone(timezone.utc).isoformat()


def _loads_json(value: str, field: str) -> Any:
    try:
        return json.loads(value)
    except json.JSONDecodeError as exc:
        raise ValueError(f"Invalid JSON for {field}: {exc}") from exc



def _list(client: APIClient, as_json: bool, _: Dict[str, object]) -> None:
    results = client.get("/api/evaluations")
    if as_json:
        print(dumps_json(results))
        return
    rows = []
    for item in results:
        if isinstance(item, dict):
            rows.append(
                {
                    "id": item.get("id"),
                    "artifact": item.get("artifact_id") or item.get("artifact"),
                    "tier": item.get("tier"),
                    "status": item.get("status"),
                    "next_refresh_at": item.get("next_refresh_at"),
                }
            )
        elif isinstance(item, (list, tuple)) and len(item) >= 4:
            rows.append(
                {
                    "id": None,
                    "artifact": item[0],
                    "tier": item[1],
                    "status": item[2],
                    "next_refresh_at": item[3],
                }
            )
    columns = ["id", "artifact", "tier", "status", "next_refresh_at"]
    print(render_table(rows, columns))


def _retry(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    result = client.post(f"/api/evaluations/{args['evaluation_id']}/retry")
    if as_json:
        print(dumps_json(result))
    else:
        columns = ["id", "status", "next_refresh_at"]
        print(render_table([result], columns))


def _status(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    data = client.get(f"/api/evaluations/{args['evaluation_id']}/status")
    if as_json:
        print(dumps_json(data))
        return
    columns = [
        "id",
        "status",
        "tier",
        "policy_requirement",
        "next_refresh_at",
        "refresh_cadence_seconds",
        "governance_notes",
    ]
    print(render_table([data], columns))


def _lineage(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    data = client.get(f"/api/evaluations/{args['evaluation_id']}/lineage")
    if as_json:
        print(dumps_json(data))
        return
    columns = ["valid_from", "valid_until", "evidence_lineage"]
    print(render_table([data], columns))


def _plan(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    payload: Dict[str, Any] = {}
    cadence = args.get("cadence_seconds")
    if cadence is not None:
        if cadence <= 0:
            raise ValueError("--cadence-seconds must be positive")
        payload["refresh_cadence_seconds"] = cadence
    if args.get("unset_cadence"):
        payload["refresh_cadence_seconds"] = None

    next_refresh = args.get("next_refresh")
    if next_refresh:
        payload["next_refresh_at"] = _normalize_iso(next_refresh)
    if args.get("unset_next_refresh"):
        payload["next_refresh_at"] = None

    if args.get("note"):
        payload["governance_notes"] = args["note"]
    if args.get("unset_note"):
        payload["governance_notes"] = None

    if args.get("source"):
        payload["evidence_source"] = _loads_json(args["source"], "--source")
    if args.get("unset_source"):
        payload["evidence_source"] = None

    if args.get("lineage"):
        payload["evidence_lineage"] = _loads_json(args["lineage"], "--lineage")
    if args.get("unset_lineage"):
        payload["evidence_lineage"] = None

    if not payload:
        raise ValueError("No plan overrides specified")

    result = client.patch(
        f"/api/evaluations/{args['evaluation_id']}/status",
        json=payload,
    )
    if as_json:
        print(dumps_json(result))
    else:
        columns = [
            "id",
            "status",
            "next_refresh_at",
            "refresh_cadence_seconds",
            "governance_notes",
        ]
        print(render_table([result], columns))


__all__ = [
    "install",
]
