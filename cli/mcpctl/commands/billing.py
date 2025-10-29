"""SaaS billing and subscription commands."""
# key: billing_cli -> commands

from __future__ import annotations

import sys
from argparse import ArgumentParser, _SubParsersAction
from typing import Callable, Dict, Optional

from ..client import APIClient, APIError
from ..renderers import dumps_json, render_table


def install(
    subparsers: _SubParsersAction[ArgumentParser],
    add_common_arguments: Callable[[ArgumentParser], None],
) -> None:
    parser = subparsers.add_parser("billing", help="Manage SaaS subscriptions and quotas")
    billing_sub = parser.add_subparsers(dest="billing_cmd", required=True)

    plans_parser = billing_sub.add_parser("plans", help="List billing plans")
    plans_parser.set_defaults(handler=_billing_plans)
    add_common_arguments(plans_parser)

    subscription_parser = billing_sub.add_parser(
        "subscription", help="Show the active subscription for an organization"
    )
    subscription_parser.add_argument("organization_id", type=int)
    subscription_parser.set_defaults(handler=_billing_subscription)
    add_common_arguments(subscription_parser)

    assign_parser = billing_sub.add_parser(
        "assign", help="Assign or update a plan for an organization"
    )
    assign_parser.add_argument("organization_id", type=int)
    assign_parser.add_argument("--plan-id", dest="plan_id")
    assign_parser.add_argument("--plan-code", dest="plan_code")
    assign_parser.add_argument("--status", dest="status")
    assign_parser.add_argument("--trial-ends", dest="trial_ends_at")
    assign_parser.set_defaults(handler=_billing_assign)
    add_common_arguments(assign_parser)

    quota_parser = billing_sub.add_parser(
        "quota", help="Check or record quota usage for an entitlement"
    )
    quota_parser.add_argument("organization_id", type=int)
    quota_parser.add_argument(
        "--entitlement",
        dest="entitlement_key",
        required=True,
        help="Entitlement key to evaluate",
    )
    quota_parser.add_argument(
        "--quantity",
        dest="requested_quantity",
        type=int,
        default=0,
        help="Requested quantity to evaluate (default: 0)",
    )
    quota_parser.add_argument(
        "--record",
        dest="record_usage",
        action="store_true",
        help="Record usage if the check passes",
    )
    quota_parser.set_defaults(handler=_billing_quota)
    add_common_arguments(quota_parser)


def _billing_plans(client: APIClient, as_json: bool, _: Dict[str, object]) -> None:
    try:
        plans = client.get("/api/billing/plans")
    except APIError as exc:
        _report_error("plans", exc)
        return

    if as_json:
        print(dumps_json(plans))
        return

    records = plans if isinstance(plans, list) else []
    print(
        render_table(
            records,
            columns=[
                ("id", "Plan ID"),
                ("code", "Code"),
                ("name", "Name"),
                ("billing_period", "Period"),
                ("amount_cents", "Amount (¢)"),
            ],
        )
    )


def _billing_subscription(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    organization_id = args["organization_id"]
    try:
        payload = client.get(f"/api/billing/organizations/{organization_id}/subscription")
    except APIError as exc:
        _report_error("subscription", exc)
        return

    if as_json:
        print(dumps_json(payload))
        return

    if not payload:
        print("No active subscription found.")
        return

    subscription = payload.get("subscription", {})
    plan = payload.get("plan", {})
    print(
        render_table(
            [
                {
                    "subscription_id": subscription.get("id"),
                    "plan": plan.get("name"),
                    "status": subscription.get("status"),
                    "current_period_end": subscription.get("current_period_end"),
                }
            ],
            columns=[
                ("subscription_id", "Subscription"),
                ("plan", "Plan"),
                ("status", "Status"),
                ("current_period_end", "Period End"),
            ],
        )
    )


def _billing_assign(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    organization_id = args["organization_id"]
    plan_id = args.get("plan_id")
    plan_code = args.get("plan_code")

    if not plan_id and plan_code:
        plan_id = _resolve_plan_id(client, plan_code)
        if not plan_id:
            print(
                f"Plan with code '{plan_code}' was not found. Use `mcpctl billing plans` to list options.",
                file=sys.stderr,
            )
            return

    if not plan_id:
        print(
            "A plan identifier or code is required. Use --plan-id or --plan-code.",
            file=sys.stderr,
        )
        return

    payload = {"plan_id": plan_id}
    status = args.get("status")
    if status:
        payload["status"] = status
    trial_ends_at = args.get("trial_ends_at")
    if trial_ends_at:
        payload["trial_ends_at"] = trial_ends_at

    try:
        response = client.post(
            f"/api/billing/organizations/{organization_id}/subscription",
            json_body=payload,
        )
    except APIError as exc:
        _report_error("assign", exc)
        return

    if as_json:
        print(dumps_json(response))
        return

    subscription = response.get("subscription", {})
    plan = response.get("plan", {})
    print(
        render_table(
            [
                {
                    "subscription_id": subscription.get("id"),
                    "plan": plan.get("name"),
                    "status": subscription.get("status"),
                }
            ],
            columns=[
                ("subscription_id", "Subscription"),
                ("plan", "Plan"),
                ("status", "Status"),
            ],
        )
    )


def _billing_quota(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    organization_id = args["organization_id"]
    payload = {
        "entitlement_key": args["entitlement_key"],
        "requested_quantity": args.get("requested_quantity", 0),
        "record_usage": bool(args.get("record_usage")),
    }

    try:
        response = client.post(
            f"/api/billing/organizations/{organization_id}/quotas/check",
            json_body=payload,
        )
    except APIError as exc:
        _report_error("quota", exc)
        return

    if as_json:
        print(dumps_json(response))
        return

    outcome = response.get("outcome", {})
    print(
        render_table(
            [
                {
                    "entitlement": outcome.get("entitlement_key"),
                    "allowed": outcome.get("allowed"),
                    "limit": outcome.get("limit_quantity"),
                    "used": outcome.get("used_quantity"),
                    "remaining": outcome.get("remaining_quantity"),
                    "notes": ", ".join(outcome.get("notes", [])),
                    "recorded": response.get("recorded"),
                }
            ],
            columns=[
                ("entitlement", "Entitlement"),
                ("allowed", "Allowed"),
                ("limit", "Limit"),
                ("used", "Used"),
                ("remaining", "Remaining"),
                ("notes", "Notes"),
                ("recorded", "Recorded"),
            ],
        )
    )


def _resolve_plan_id(client: APIClient, plan_code: str) -> Optional[str]:
    try:
        plans = client.get("/api/billing/plans")
    except APIError as exc:
        _report_error("plans", exc)
        return None

    if not isinstance(plans, list):
        return None

    for plan in plans:
        if plan.get("code") == plan_code:
            return plan.get("id")
    return None


def _report_error(operation: str, error: APIError) -> None:
    print(
        f"Billing {operation} failed: {error.status_code} {error.message}",
        file=sys.stderr,
    )
