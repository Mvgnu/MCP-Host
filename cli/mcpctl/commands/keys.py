"""Provider BYOK key management commands."""
# key: keys_cli -> commands

from __future__ import annotations

import json
import sys
import base64
import hashlib
from argparse import ArgumentParser, _SubParsersAction
from pathlib import Path
from typing import Callable, Dict

from ..client import APIClient, APIError
from ..renderers import dumps_json, render_table


def install(
    subparsers: _SubParsersAction[ArgumentParser],
    add_common_arguments: Callable[[ArgumentParser], None],
) -> None:
    parser = subparsers.add_parser(
        "keys", help="Provider BYOK key registration, rotation, and posture"
    )
    keys_sub = parser.add_subparsers(dest="keys_cmd", required=True)

    register_parser = keys_sub.add_parser(
        "register", help="Register a provider key using an attestation bundle"
    )
    register_parser.add_argument("provider_id", help="Provider identifier")
    register_parser.add_argument("--alias", help="Friendly label for the key")
    register_parser.add_argument(
        "--attestation",
        help="Path to attestation bundle (placeholder until backend upload surfaces)",
        required=True,
    )
    register_parser.add_argument(
        "--rotation-due",
        dest="rotation_due",
        help="Rotation due timestamp (RFC 3339)",
    )
    register_parser.set_defaults(handler=_keys_register)
    add_common_arguments(register_parser)

    list_parser = keys_sub.add_parser(
        "list", help="List BYOK entries for a provider"
    )
    list_parser.add_argument("provider_id", help="Provider identifier")
    list_parser.set_defaults(handler=_keys_list)
    add_common_arguments(list_parser)

    rotate_parser = keys_sub.add_parser(
        "rotate", help="Request a key rotation"
    )
    rotate_parser.add_argument("provider_id", help="Provider identifier")
    rotate_parser.add_argument("key_id", help="Key identifier")
    rotate_parser.add_argument(
        "--attestation",
        help="Rotation attestation bundle (placeholder)",
        required=True,
    )
    rotate_parser.add_argument(
        "--actor-ref",
        dest="actor_ref",
        help="Operator or system reference requesting rotation",
        required=True,
    )
    rotate_parser.set_defaults(handler=_keys_rotate)
    add_common_arguments(rotate_parser)

    revoke_parser = keys_sub.add_parser(
        "revoke", help="Emergency revoke a provider key"
    )
    revoke_parser.add_argument("provider_id", help="Provider identifier")
    revoke_parser.add_argument("key_id", help="Key identifier")
    revoke_parser.add_argument("--reason", help="Optional operator supplied reason")
    revoke_parser.add_argument("--compromised", dest="mark_compromised", action="store_true", help="Mark the key as compromised (defaults to retiring)")
    revoke_parser.set_defaults(handler=_keys_revoke)
    add_common_arguments(revoke_parser)

    approve_parser = keys_sub.add_parser(
        "approve-rotation", help="Approve a pending key rotation (stub handler)"
    )
    approve_parser.add_argument("provider_id", help="Provider identifier")
    approve_parser.add_argument("rotation_id", help="Rotation request identifier")
    approve_parser.set_defaults(handler=_not_yet_available, operation="approve-rotation")
    add_common_arguments(approve_parser)

    bind_parser = keys_sub.add_parser(
        "bind", help="Attach a provider key to a workload scope"
    )
    bind_parser.add_argument("provider_id", help="Provider identifier")
    bind_parser.add_argument("key_id", help="Key identifier")
    bind_parser.add_argument(
        "--type",
        dest="binding_type",
        required=True,
        help="Binding type (e.g., workspace, runtime)",
    )
    bind_parser.add_argument(
        "--target",
        dest="binding_target_id",
        required=True,
        help="Target identifier for the binding",
    )
    bind_parser.add_argument(
        "--context",
        help="Optional JSON context describing the binding",
    )
    bind_parser.set_defaults(handler=_keys_bind)
    add_common_arguments(bind_parser)

    bindings_parser = keys_sub.add_parser(
        "bindings", help="List key bindings"
    )
    bindings_parser.add_argument("provider_id", help="Provider identifier")
    bindings_parser.add_argument("key_id", help="Key identifier")
    bindings_parser.set_defaults(handler=_keys_bindings)
    add_common_arguments(bindings_parser)

    audit_parser = keys_sub.add_parser(
        "audit", help="Query BYOK audit events"
    )
    audit_parser.add_argument("provider_id", help="Provider identifier")
    audit_parser.add_argument("--key-id", dest="key_id", help="Filter by provider key identifier")
    audit_parser.add_argument("--state", help="Filter by posture state")
    audit_parser.add_argument("--start", help="Filter events occurring at or after this RFC3339 timestamp")
    audit_parser.add_argument("--end", help="Filter events occurring before this RFC3339 timestamp")
    audit_parser.add_argument("--limit", type=int, help="Maximum number of events to fetch")
    audit_parser.set_defaults(handler=_keys_audit)
    add_common_arguments(audit_parser)

    watch_parser = keys_sub.add_parser(
        "watch", help="Stream provider key posture via SSE (stub handler)"
    )
    watch_parser.add_argument("provider_id", help="Provider identifier")
    watch_parser.set_defaults(handler=_not_yet_available, operation="watch")
    add_common_arguments(watch_parser)


def _keys_register(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    provider_id = args["provider_id"]
    payload = {"alias": args.get("alias")}
    attestation_path = args.get("attestation")
    if not attestation_path:
        print("An attestation bundle is required for BYOK registration.", file=sys.stderr)
        return

    bundle = _load_attestation_bundle(str(attestation_path))
    payload.update(bundle)

    rotation_due = args.get("rotation_due")
    if rotation_due:
        payload["rotation_due_at"] = rotation_due
    try:
        record = client.post(
            f"/api/providers/{provider_id}/keys", json_body=payload
        )
    except APIError as exc:
        _report_stubbed_feature("register", exc)
        return

    if as_json:
        print(dumps_json(record))
    else:
        print(
            render_table(
                [record],
                columns=[
                    ("id", "Key ID"),
                    ("alias", "Alias"),
                    ("state", "State"),
                    ("rotation_due_at", "Rotation Due"),
                    ("activated_at", "Activated"),
                ],
            )
        )


def _keys_list(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    provider_id = args["provider_id"]
    try:
        payload = client.get(f"/api/providers/{provider_id}/keys")
    except APIError as exc:
        _report_stubbed_feature("list", exc)
        return

    if as_json:
        print(dumps_json(payload))
        return

    records = payload if isinstance(payload, list) else []
    print(
        render_table(
            records,
            columns=[
                ("id", "Key ID"),
                ("alias", "Alias"),
                ("state", "State"),
                ("rotation_due_at", "Rotation Due"),
                ("activated_at", "Activated"),
            ],
        )
    )


def _keys_rotate(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    provider_id = args["provider_id"]
    key_id = args["key_id"]
    payload: Dict[str, object] = {}

    attestation_path = args.get("attestation")
    if not attestation_path:
        print(
            "Rotation attestation bundle is required when requesting BYOK rotation.",
            file=sys.stderr,
        )
        return

    payload.update(_load_attestation_bundle(str(attestation_path)))

    actor_ref = args.get("actor_ref")
    if not actor_ref:
        print("Rotation actor reference is required for audit logging.", file=sys.stderr)
        return

    payload["request_actor_ref"] = actor_ref

    try:
        rotation = client.post(
            f"/api/providers/{provider_id}/keys/{key_id}/rotations",
            json_body=payload,
        )
    except APIError as exc:
        _report_stubbed_feature("rotate", exc)
        return

    if as_json:
        print(dumps_json(rotation))
        return

    print(
        render_table(
            [rotation],
            columns=[
                ("id", "Rotation ID"),
                ("status", "Status"),
                ("requested_at", "Requested"),
                ("request_actor_ref", "Actor"),
            ],
        )
    )


def _keys_revoke(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    provider_id = args["provider_id"]
    key_id = args["key_id"]
    payload: Dict[str, object] = {"mark_compromised": bool(args.get("mark_compromised"))}
    reason = args.get("reason")
    if reason:
        payload["reason"] = reason

    try:
        record = client.post(
            f"/api/providers/{provider_id}/keys/{key_id}/revocations",
            json_body=payload,
        )
    except APIError as exc:
        _report_stubbed_feature("revoke", exc)
        return

    if as_json:
        print(dumps_json(record))
        return

    print(
        render_table(
            [record],
            columns=[
                ("id", "Key ID"),
                ("state", "State"),
                ("retired_at", "Retired"),
                ("compromised_at", "Compromised"),
            ],
        )
    )


def _keys_audit(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    provider_id = args["provider_id"]
    params: Dict[str, object] = {}
    if args.get("key_id"):
        params["key_id"] = args["key_id"]
    if args.get("state"):
        params["state"] = args["state"]
    if args.get("start"):
        params["start"] = args["start"]
    if args.get("end"):
        params["end"] = args["end"]
    if args.get("limit") is not None:
        params["limit"] = args["limit"]

    try:
        payload = client.get(
            f"/api/providers/{provider_id}/keys/audit", params=params or None
        )
    except APIError as exc:
        _report_stubbed_feature("audit", exc)
        return

    if as_json:
        print(dumps_json(payload))
        return

    entries = payload if isinstance(payload, list) else []
    normalized = []
    for entry in entries:
        event = entry.get("event", {})
        normalized.append(
            {
                "event_id": event.get("id"),
                "key_id": event.get("provider_key_id"),
                "event_type": event.get("event_type"),
                "occurred_at": event.get("occurred_at"),
                "state": entry.get("provider_key_state"),
            }
        )

    print(
        render_table(
            normalized,
            columns=[
                ("event_id", "Event ID"),
                ("key_id", "Key ID"),
                ("event_type", "Type"),
                ("occurred_at", "Occurred"),
                ("state", "State"),
            ],
        )
    )


def _keys_bind(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    provider_id = args["provider_id"]
    key_id = args["key_id"]
    binding_type = args.get("binding_type")
    target_id = args.get("binding_target_id")

    if not binding_type or not target_id:
        print(
            "Binding type and target are required to attach a provider key.",
            file=sys.stderr,
        )
        return

    context_raw = args.get("context")
    additional_context: Dict[str, object]
    if context_raw:
        try:
            parsed = json.loads(str(context_raw))
            if not isinstance(parsed, dict):
                raise ValueError("Binding context must be a JSON object")
            additional_context = parsed
        except ValueError as exc:
            print(f"Invalid binding context: {exc}", file=sys.stderr)
            return
    else:
        additional_context = {}

    payload: Dict[str, object] = {
        "binding_type": binding_type,
        "binding_target_id": target_id,
    }
    if additional_context:
        payload["additional_context"] = additional_context

    try:
        record = client.post(
            f"/api/providers/{provider_id}/keys/{key_id}/bindings",
            json_body=payload,
        )
    except APIError as exc:
        _report_stubbed_feature("bind", exc)
        return

    if as_json:
        print(dumps_json(record))
        return

    print(
        render_table(
            [record],
            columns=[
                ("id", "Binding ID"),
                ("binding_type", "Type"),
                ("binding_target_id", "Target"),
                ("created_at", "Created"),
            ],
        )
    )


def _keys_bindings(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    provider_id = args["provider_id"]
    key_id = args["key_id"]

    try:
        payload = client.get(
            f"/api/providers/{provider_id}/keys/{key_id}/bindings"
        )
    except APIError as exc:
        _report_stubbed_feature("bindings", exc)
        return

    records = payload if isinstance(payload, list) else []
    if as_json:
        print(dumps_json(records))
        return

    print(
        render_table(
            records,
            columns=[
                ("id", "Binding ID"),
                ("binding_type", "Type"),
                ("binding_target_id", "Target"),
                ("created_at", "Created"),
            ],
        )
    )


def _not_yet_available(
    _client: APIClient, _as_json: bool, args: Dict[str, object]
) -> None:
    command = args.get("operation", "operation")
    print(
        f"Provider key {command} is not yet available. Backend endpoints will surface in a follow-up.",
        file=sys.stderr,
    )


def _report_stubbed_feature(operation: str, exc: APIError) -> None:
    if exc.status_code == 501:
        print(
            f"Provider key {operation} is not yet implemented on the API surface (HTTP 501).",
            file=sys.stderr,
        )
    else:
        raise


def _load_attestation_bundle(path: str) -> Dict[str, str]:
    data = Path(path).read_bytes()
    digest = hashlib.sha256(data).digest()
    return {
        "attestation_digest": base64.b64encode(digest).decode("ascii"),
        "attestation_signature": base64.b64encode(data).decode("ascii"),
    }
