"""Entry-point wiring for the mission-control CLI."""
# key: operator-cli -> parser

from __future__ import annotations

import os
import sys
from argparse import ArgumentParser
from typing import Any, Dict

from .client import APIClient, APIError
from .commands import (
    install_evaluations,
    install_governance,
    install_marketplace,
    install_policy,
    install_promotions,
    install_scaffold,
    install_trust,
)

ENV_HOST = "MCP_HOST_URL"
ENV_TOKEN = "MCP_HOST_TOKEN"


def build_parser() -> ArgumentParser:
    parser = ArgumentParser(prog="mcpctl", description="Mission-control CLI for MCP Host")
    parser.add_argument(
        "--host",
        default=os.environ.get(ENV_HOST, "http://localhost:3000"),
        help="Base URL for the MCP Host API",
    )
    parser.add_argument(
        "--token",
        default=os.environ.get(ENV_TOKEN),
        help="Bearer token used for authentication (env: MCP_HOST_TOKEN)",
    )
    parser.add_argument("--timeout", type=int, default=30, help="HTTP timeout in seconds")

    subparsers = parser.add_subparsers(dest="command", required=True)
    install_marketplace(subparsers)
    install_policy(subparsers)
    install_promotions(subparsers)
    install_governance(subparsers)
    install_evaluations(subparsers)
    install_trust(subparsers)
    install_scaffold(subparsers)
    return parser


def dispatch(args_namespace: Any) -> int:
    handler = getattr(args_namespace, "handler", None)
    if handler is None:
        raise SystemExit("No command handler bound")

    client = APIClient(
        base_url=args_namespace.host,
        token=args_namespace.token,
        timeout=args_namespace.timeout,
    )

    arguments = _extract_arguments(args_namespace)
    as_json = arguments.pop("json", False)

    try:
        handler(client, as_json, arguments)
        return 0
    except APIError as exc:
        print(f"API error ({exc.status_code}): {exc}", file=sys.stderr)
        if exc.payload is not None and as_json:
            print(exc.payload, file=sys.stderr)
        return exc.status_code
    except ValueError as exc:
        print(f"Error: {exc}", file=sys.stderr)
        return 1


def _extract_arguments(namespace: Any) -> Dict[str, Any]:
    raw = vars(namespace).copy()
    for meta_key in list(raw):
        if meta_key in {"handler", "host", "token", "timeout", "command"}:
            raw.pop(meta_key, None)
        elif meta_key.endswith("_cmd"):
            raw.pop(meta_key, None)
    return raw


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    return dispatch(args)


if __name__ == "__main__":  # pragma: no cover - CLI entry
    sys.exit(main())
