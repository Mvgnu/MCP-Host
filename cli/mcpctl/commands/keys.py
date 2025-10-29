"""Provider BYOK key management commands."""
# key: keys_cli -> commands

from __future__ import annotations

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

    approve_parser = keys_sub.add_parser(
        "approve-rotation", help="Approve a pending key rotation (stub handler)"
    )
    approve_parser.add_argument("provider_id", help="Provider identifier")
    approve_parser.add_argument("rotation_id", help="Rotation request identifier")
    approve_parser.set_defaults(handler=_not_yet_available, operation="approve-rotation")
    add_common_arguments(approve_parser)

    bindings_parser = keys_sub.add_parser(
        "bindings", help="List key bindings (stub handler)"
    )
    bindings_parser.add_argument("provider_id", help="Provider identifier")
    bindings_parser.add_argument("key_id", help="Key identifier")
    bindings_parser.set_defaults(handler=_not_yet_available, operation="bindings")
    add_common_arguments(bindings_parser)

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

    render_table(
        [rotation],
        columns=[
            ("id", "Rotation ID"),
            ("status", "Status"),
            ("requested_at", "Requested"),
            ("request_actor_ref", "Actor"),
        ],
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
