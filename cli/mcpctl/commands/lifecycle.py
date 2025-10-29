"""Lifecycle console streaming and snapshot commands."""
# key: lifecycle-cli -> commands

from __future__ import annotations

import json
from argparse import ArgumentParser, _SubParsersAction
from typing import Any, Callable, Dict, Iterable, Mapping, MutableMapping, Optional

from ..client import APIClient
from ..renderers import dumps_json, render_table

CommandFn = Callable[[APIClient, bool, Dict[str, object]], None]


def install(
    subparsers: _SubParsersAction[ArgumentParser],
    add_common_arguments: Callable[[ArgumentParser], None],
) -> None:
    parser = subparsers.add_parser(
        "lifecycle",
        help="Lifecycle console snapshots and streaming automation context",
    )
    lifecycle_sub = parser.add_subparsers(dest="lifecycle_cmd", required=True)

    list_parser = lifecycle_sub.add_parser(
        "list",
        help="List lifecycle console snapshots with promotion automation context",
    )
    _add_query_arguments(list_parser)
    list_parser.set_defaults(handler=_lifecycle_list)
    add_common_arguments(list_parser)

    watch_parser = lifecycle_sub.add_parser(
        "watch",
        help="Stream lifecycle console snapshots and deltas via SSE",
    )
    _add_query_arguments(watch_parser)
    watch_parser.add_argument(
        "--heartbeat-ms",
        dest="heartbeat_ms",
        type=int,
        help="Heartbeat interval for SSE keep-alive messages",
    )
    watch_parser.set_defaults(handler=_lifecycle_watch)
    add_common_arguments(watch_parser)


def _add_query_arguments(parser: ArgumentParser) -> None:
    parser.add_argument("--cursor", type=int, help="Resume snapshots from the supplied cursor")
    parser.add_argument("--limit", type=int, help="Maximum number of workspaces to return")
    parser.add_argument(
        "--lifecycle-state",
        dest="lifecycle_state",
        help="Filter workspaces by lifecycle state",
    )
    parser.add_argument("--owner-id", dest="owner_id", type=int, help="Filter by workspace owner")
    parser.add_argument(
        "--workspace-key",
        dest="workspace_key",
        help="Filter by an exact workspace key",
    )
    parser.add_argument(
        "--workspace-search",
        dest="workspace_search",
        help="Case-insensitive search across workspace names",
    )
    parser.add_argument(
        "--promotion-lane",
        dest="promotion_lane",
        help="Filter promotion runs by lane identifier",
    )
    parser.add_argument(
        "--severity",
        dest="severity",
        help="Filter snapshots by blended severity classification",
    )
    parser.add_argument(
        "--run-limit",
        dest="run_limit",
        type=int,
        help="Bound the number of recent remediation runs per workspace",
    )


def _collect_params(args: Mapping[str, object]) -> MutableMapping[str, object]:
    params: MutableMapping[str, object] = {}
    for key in (
        "cursor",
        "limit",
        "lifecycle_state",
        "owner_id",
        "workspace_key",
        "workspace_search",
        "promotion_lane",
        "severity",
        "run_limit",
    ):
        value = args.get(key)
        if value is not None:
            params[key] = value
    return params


def _lifecycle_list(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    params = _collect_params(args)
    page = client.get("/api/console/lifecycle", params=params or None)
    if as_json:
        print(dumps_json(page))
        return
    _render_page(page)


def _lifecycle_watch(client: APIClient, as_json: bool, args: Dict[str, object]) -> None:
    params = _collect_params(args)
    heartbeat = args.get("heartbeat_ms")
    if heartbeat is not None:
        params["heartbeat_ms"] = heartbeat
    try:
        for payload in client.stream_sse(
            "/api/console/lifecycle/stream", params=params or None
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


def _render_event(event: Mapping[str, Any]) -> None:
    event_type = event.get("type")
    cursor = event.get("cursor")
    emitted_at = event.get("emitted_at")
    if event_type == "heartbeat":
        print(f"[{emitted_at}] lifecycle heartbeat (cursor={cursor})")
        return
    if event_type == "error":
        print(f"[{emitted_at}] lifecycle error: {event.get('error')}")
        return
    page = event.get("page") if isinstance(event.get("page"), Mapping) else None
    if page:
        print(f"[{emitted_at}] lifecycle snapshot (cursor={cursor})")
        _render_page(page)
    delta = event.get("delta")
    if isinstance(delta, Mapping):
        _render_delta(delta)


def _render_page(page: Mapping[str, Any]) -> None:
    workspaces = page.get("workspaces")
    if not isinstance(workspaces, Iterable):
        print("No lifecycle workspaces available")
        return
    for snapshot in workspaces:
        if not isinstance(snapshot, Mapping):
            continue
        _render_workspace(snapshot)


def _render_workspace(snapshot: Mapping[str, Any]) -> None:
    workspace = snapshot.get("workspace")
    if not isinstance(workspace, Mapping):
        return
    workspace_id = workspace.get("id")
    workspace_key = workspace.get("workspace_key")
    state = workspace.get("lifecycle_state")
    owner = workspace.get("owner_id")
    display = workspace.get("display_name")
    print(
        f"workspace {workspace_id} ({workspace_key}) "
        f"state={state} owner={owner} name={display}"
    )
    promotion_runs = snapshot.get("promotion_runs")
    if isinstance(promotion_runs, Iterable):
        rows = []
        for item in promotion_runs:
            if not isinstance(item, Mapping):
                continue
            gate_context = item.get("promotion_gate_context")
            if not isinstance(gate_context, Mapping):
                gate_context = {}
            lane = _extract_string(gate_context, ("lane", "promotion_lane"))
            stage = _extract_string(gate_context, ("stage", "promotion_stage"))
            rows.append(
                {
                    "id": item.get("id"),
                    "status": item.get("status"),
                    "playbook": item.get("playbook"),
                    "lane": lane or "-",
                    "stage": stage or "-",
                    "payload": _summarize_json(item.get("automation_payload")),
                    "metadata": _summarize_json(item.get("metadata")),
                }
            )
        if rows:
            print("Promotion automation runs:")
            print(
                render_table(
                    rows,
                    ["id", "status", "playbook", "lane", "stage", "payload", "metadata"],
                )
            )
    promotion_postures = snapshot.get("promotion_postures")
    if isinstance(promotion_postures, Iterable):
        rows = []
        for posture in promotion_postures:
            if not isinstance(posture, Mapping):
                continue
            rows.append(
                {
                    "promotion": posture.get("promotion_id"),
                    "stage": posture.get("stage"),
                    "track": posture.get("track_name"),
                    "tier": posture.get("track_tier"),
                    "status": posture.get("status"),
                    "allowed": "yes" if posture.get("allowed") else "no",
                    "updated": posture.get("updated_at"),
                }
            )
        if rows:
            print("Promotion posture verdicts:")
            print(
                render_table(
                    rows,
                    ["promotion", "stage", "track", "tier", "status", "allowed", "updated"],
                )
            )
    recent_runs = snapshot.get("recent_runs")
    if isinstance(recent_runs, Iterable):
        rows = []
        for run in recent_runs:
            if not isinstance(run, Mapping):
                continue
            run_body = run.get("run") if isinstance(run.get("run"), Mapping) else {}
            rows.append(
                {
                    "id": run_body.get("id"),
                    "status": run_body.get("status"),
                    "playbook": run_body.get("playbook"),
                    "attempt": _summarize_attempt(
                        run.get("retry_attempt"), run.get("retry_limit")
                    ),
                    "duration": _summarize_duration(run.get("duration_seconds")),
                    "override": run.get("override_reason") or "-",
                    "trust": _summarize_trust(run.get("trust")),
                    "market": _summarize_marketplace(run.get("marketplace")),
                    "artifacts": _summarize_artifacts(run.get("artifacts")),
                }
            )
        if rows:
            print("Recent remediation runs:")
            print(
                render_table(
                    rows,
                    [
                        "id",
                        "status",
                        "playbook",
                        "attempt",
                        "duration",
                        "override",
                        "trust",
                        "market",
                        "artifacts",
                    ],
                )
            )
    print("")


def _render_delta(delta: Mapping[str, Any]) -> None:
    workspaces = delta.get("workspaces")
    if not isinstance(workspaces, Iterable):
        return
    for workspace_delta in workspaces:
        if not isinstance(workspace_delta, Mapping):
            continue
        workspace_id = workspace_delta.get("workspace_id")
        run_deltas = workspace_delta.get("run_deltas")
        if isinstance(run_deltas, Iterable) and not isinstance(run_deltas, (str, bytes)):
            for run_delta in run_deltas:
                if not isinstance(run_delta, Mapping):
                    continue
                run_id = run_delta.get("run_id")
                status = run_delta.get("status")
                changes = _summarize_field_changes(
                    run_delta,
                    (
                        "trust_changes",
                        "intelligence_changes",
                        "marketplace_changes",
                        "analytics_changes",
                        "artifact_changes",
                    ),
                )
                print(f"workspace {workspace_id} run {run_id} -> status={status}{changes}")
        removed_runs = workspace_delta.get("removed_run_ids")
        if isinstance(removed_runs, Iterable) and not isinstance(removed_runs, (str, bytes)):
            for run_id in removed_runs:
                print(f"workspace {workspace_id} run {run_id} removed")
        promotion_deltas = workspace_delta.get("promotion_run_deltas")
        if isinstance(promotion_deltas, Iterable) and not isinstance(promotion_deltas, (str, bytes)):
            for run_delta in promotion_deltas:
                if not isinstance(run_delta, Mapping):
                    continue
                run_id = run_delta.get("run_id")
                status = run_delta.get("status")
                changes = _summarize_field_changes(run_delta)
                print(
                    f"workspace {workspace_id} promotion-run {run_id} -> status={status}{changes}"
                )
        removed_runs = workspace_delta.get("removed_promotion_run_ids")
        if isinstance(removed_runs, Iterable) and not isinstance(removed_runs, (str, bytes)):
            for run_id in removed_runs:
                print(f"workspace {workspace_id} promotion-run {run_id} removed")
        promotion_postures = workspace_delta.get("promotion_posture_deltas")
        if isinstance(promotion_postures, Iterable) and not isinstance(
            promotion_postures, (str, bytes)
        ):
            for posture in promotion_postures:
                if not isinstance(posture, Mapping):
                    continue
                promotion_id = posture.get("promotion_id")
                status = posture.get("status")
                allowed = posture.get("allowed")
                stage = posture.get("stage")
                track = posture.get("track_name")
                tier = posture.get("track_tier")
                details: list[str] = []
                if stage:
                    details.append(f"stage={stage}")
                if track:
                    details.append(f"track={track}")
                if tier:
                    details.append(f"tier={tier}")
                veto_reasons = posture.get("veto_reasons")
                if isinstance(veto_reasons, Iterable) and not isinstance(
                    veto_reasons, (str, bytes)
                ):
                    rendered = ", ".join(str(reason) for reason in veto_reasons if reason)
                    if rendered:
                        details.append(f"veto=[{rendered}]")
                notes = posture.get("notes")
                if isinstance(notes, Iterable) and not isinstance(notes, (str, bytes)):
                    rendered = ", ".join(str(note) for note in notes if note)
                    if rendered:
                        details.append(f"notes=[{rendered}]")
                hooks = posture.get("remediation_hooks")
                if isinstance(hooks, Iterable) and not isinstance(hooks, (str, bytes)):
                    rendered = ", ".join(str(hook) for hook in hooks if hook)
                    if rendered:
                        details.append(f"hooks=[{rendered}]")
                signals = posture.get("signals")
                if signals:
                    details.append(f"signals={_summarize_json(signals)}")
                allowed_flag = "yes" if bool(allowed) else "no"
                suffix = f" {' '.join(details)}" if details else ""
                print(
                    f"workspace {workspace_id} promotion {promotion_id} -> status={status} allowed={allowed_flag}{suffix}"
                )
        removed_promotions = workspace_delta.get("removed_promotion_ids")
        if isinstance(removed_promotions, Iterable) and not isinstance(
            removed_promotions, (str, bytes)
        ):
            for promotion_id in removed_promotions:
                print(f"workspace {workspace_id} promotion {promotion_id} removed")


def _summarize_field_changes(
    run_delta: Mapping[str, Any],
    keys: Iterable[str] | None = None,
) -> str:
    summaries: list[str] = []
    if keys is None:
        keys = (
            "automation_payload_changes",
            "gate_context_changes",
            "metadata_changes",
            "analytics_changes",
            "artifact_changes",
        )
    for key in keys:
        changes = run_delta.get(key)
        if not isinstance(changes, Iterable) or isinstance(changes, (str, bytes)):
            continue
        rendered = []
        for change in changes:
            if not isinstance(change, Mapping):
                continue
            field = change.get("field")
            current = change.get("current")
            previous = change.get("previous")
            rendered.append(
                f"{field}={current!r} (was {previous!r})"
                if previous is not None
                else f"{field}={current!r}"
            )
        if rendered:
            summaries.append(f" {key} -> {', '.join(rendered)}")
    return "".join(summaries)


def _summarize_json(value: Any, max_length: int = 64) -> str:
    if value is None:
        return "-"
    text = json.dumps(value, sort_keys=True)
    if len(text) <= max_length:
        return text
    return text[: max_length - 3] + "..."


def _summarize_trust(value: Any) -> str:
    if not isinstance(value, Mapping):
        return "-"
    status = value.get("attestation_status") or value.get("lifecycle_state")
    lifecycle = value.get("lifecycle_state")
    if status and lifecycle:
        return f"{status}/{lifecycle}"
    return status or lifecycle or "-"


def _summarize_marketplace(value: Any) -> str:
    if not isinstance(value, Mapping):
        return "-"
    status = value.get("status")
    completed = value.get("last_completed_at")
    if status and completed:
        return f"{status} @ {completed}"
    return status or "-"


def _summarize_duration(value: Any) -> str:
    if isinstance(value, (int, float)):
        return f"{int(value)}s"
    return "-"


def _summarize_attempt(attempt: Any, retry_limit: Any) -> str:
    attempt_value = attempt if isinstance(attempt, (int, float)) else None
    limit_value = retry_limit if isinstance(retry_limit, (int, float)) else None
    if attempt_value is None and limit_value is None:
        return "-"
    if limit_value is None:
        return f"{int(attempt_value)}"
    if attempt_value is None:
        return f"-/ {int(limit_value)}"
    return f"{int(attempt_value)}/{int(limit_value)}"


def _summarize_artifacts(value: Any) -> str:
    if not isinstance(value, Iterable):
        return "-"
    entries = []
    for artifact in value:
        if not isinstance(artifact, Mapping):
            continue
        digest = artifact.get("manifest_digest")
        lane = artifact.get("lane")
        stage = artifact.get("stage")
        tag = artifact.get("manifest_tag")
        summary = str(digest) if digest else "artifact"
        details = []
        if lane:
            details.append(f"lane={lane}")
        if stage:
            details.append(f"stage={stage}")
        if tag:
            details.append(f"tag={tag}")
        if details:
            summary = f"{summary} ({', '.join(details)})"
        entries.append(summary)
    if not entries:
        return "-"
    return "; ".join(entries)


def _extract_string(data: Mapping[str, Any], keys: Iterable[str]) -> Optional[str]:
    for key in keys:
        value = data.get(key)
        if isinstance(value, str) and value:
            return value
    return None

