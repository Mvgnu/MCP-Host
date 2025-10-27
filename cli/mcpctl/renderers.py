"""Output formatting utilities for mission-control commands."""
# key: operator-cli -> renderers

from __future__ import annotations

import json
from typing import Iterable, Mapping, Sequence

Row = Mapping[str, object]


def to_rows(items: Iterable[Mapping[str, object]], columns: Sequence[str]) -> list[list[str]]:
    table: list[list[str]] = []
    for item in items:
        table.append([_stringify(item.get(column, "")) for column in columns])
    return table


def render_table(items: Iterable[Mapping[str, object]], columns: Sequence[str]) -> str:
    rows = to_rows(items, columns)
    widths = [len(column) for column in columns]
    for row in rows:
        for idx, cell in enumerate(row):
            widths[idx] = max(widths[idx], len(cell))
    header = " ".join(column.ljust(widths[idx]) for idx, column in enumerate(columns))
    separator = " ".join("-" * widths[idx] for idx in range(len(columns)))
    lines = [header, separator]
    for row in rows:
        lines.append(" ".join(cell.ljust(widths[idx]) for idx, cell in enumerate(row)))
    return "\n".join(lines)


def dumps_json(data: object) -> str:
    return json.dumps(data, indent=2, sort_keys=True)


def _stringify(value: object) -> str:
    if isinstance(value, (list, tuple)):
        return ", ".join(_stringify(item) for item in value)
    if isinstance(value, dict):
        return json.dumps(value, sort_keys=True)
    return str(value)
