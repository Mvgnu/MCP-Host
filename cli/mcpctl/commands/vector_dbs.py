from __future__ import annotations

import json
from argparse import ArgumentParser, _SubParsersAction
from typing import Callable, Dict

from ..client import APIClient, APIError
from ..renderers import dumps_json, render_table


def install(
    subparsers: _SubParsersAction[ArgumentParser],
    add_common_arguments: Callable[[ArgumentParser], None],
) -> None:
    parser = subparsers.add_parser(
        "vector-dbs", help="Manage federated vector DB governance actions"
    )
    vector_sub = parser.add_subparsers(dest="vector_dbs_cmd", required=True)

    attachments_parser = vector_sub.add_parser(
        "attachments", help="Manage vector DB attachments"
    )
    attachments_sub = attachments_parser.add_subparsers(
        dest="vector_dbs_attachments_cmd", required=True
    )

    detach_parser = attachments_sub.add_parser(
        "detach", help="Detach an attachment from a vector DB"
    )
    detach_parser.add_argument("vector_db_id", type=int)
    detach_parser.add_argument("attachment_id")
    detach_parser.add_argument(
        "--reason", help="Optional detachment reason to persist with the record"
    )
    detach_parser.set_defaults(handler=_attachments_detach)
    add_common_arguments(detach_parser)

    incidents_parser = vector_sub.add_parser(
        "incidents", help="Resolve vector DB compliance incidents"
    )
    incidents_sub = incidents_parser.add_subparsers(
        dest="vector_dbs_incidents_cmd", required=True
    )

    resolve_parser = incidents_sub.add_parser(
        "resolve", help="Resolve an incident for a vector DB"
    )
    resolve_parser.add_argument("vector_db_id", type=int)
    resolve_parser.add_argument("incident_id")
    resolve_parser.add_argument(
        "--summary",
        dest="resolution_summary",
        help="Optional resolution summary that will overwrite the current value",
    )
    resolve_parser.add_argument(
        "--notes",
        dest="resolution_notes",
        help="Resolution notes payload as JSON (object or array)",
    )
    resolve_parser.set_defaults(handler=_incidents_resolve)
    add_common_arguments(resolve_parser)


def _attachments_detach(
    client: APIClient, as_json: bool, args: Dict[str, object]
) -> None:
    vector_db_id = int(args["vector_db_id"])
    attachment_id = str(args["attachment_id"])
    payload: Dict[str, object] = {}
    reason = args.get("reason")
    if isinstance(reason, str) and reason.strip():
        payload["reason"] = reason

    try:
        record = client.patch(
            f"/api/vector-dbs/{vector_db_id}/attachments/{attachment_id}",
            json=payload,
        )
    except APIError as exc:
        _report_error("attachments detach", exc)
        return

    if as_json:
        print(dumps_json(record))
        return

    detached_at = record.get("detached_at")
    detached_reason = record.get("detached_reason") or "(not provided)"
    print(
        render_table(
            [
                {
                    "attachment_id": record.get("id"),
                    "vector_db_id": record.get("vector_db_id"),
                    "detached_at": detached_at,
                    "reason": detached_reason,
                }
            ],
            columns=["attachment_id", "vector_db_id", "detached_at", "reason"],
        )
    )


def _incidents_resolve(
    client: APIClient, as_json: bool, args: Dict[str, object]
) -> None:
    vector_db_id = int(args["vector_db_id"])
    incident_id = str(args["incident_id"])
    payload: Dict[str, object] = {}
    summary = args.get("resolution_summary")
    if isinstance(summary, str) and summary.strip():
        payload["resolution_summary"] = summary

    notes_raw = args.get("resolution_notes")
    if notes_raw is not None:
        payload["resolution_notes"] = _loads_json(notes_raw, "--notes")

    if not payload:
        raise ValueError("No resolution fields supplied; provide --summary and/or --notes")

    try:
        record = client.patch(
            f"/api/vector-dbs/{vector_db_id}/incidents/{incident_id}",
            json=payload,
        )
    except APIError as exc:
        _report_error("incidents resolve", exc)
        return

    if as_json:
        print(dumps_json(record))
        return

    print(
        render_table(
            [
                {
                    "incident_id": record.get("id"),
                    "status": "resolved",
                    "resolved_at": record.get("resolved_at"),
                    "summary": record.get("summary"),
                }
            ],
            columns=["incident_id", "status", "resolved_at", "summary"],
        )
    )


def _loads_json(value: object, flag: str) -> object:
    if isinstance(value, (dict, list)):
        return value
    if isinstance(value, str):
        try:
            return json.loads(value)
        except json.JSONDecodeError as exc:  # pragma: no cover - defensive
            raise ValueError(f"Invalid JSON payload for {flag}: {exc}") from exc
    raise ValueError(f"Unsupported value for {flag}; provide a JSON string or object")


def _report_error(operation: str, error: APIError) -> None:
    message = error.payload if isinstance(error.payload, str) else str(error)
    print(f"API error during {operation}: {message}")


__all__ = ["install"]
