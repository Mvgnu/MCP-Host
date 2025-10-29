"""Output formatting utilities for mission-control commands."""
# key: operator-cli -> renderers

from __future__ import annotations

import json
from typing import Iterable, Mapping, Sequence, Union

Row = Mapping[str, object]
Column = Union[str, tuple[str, str]]


def _column_key(column: Column) -> str:
    return column[0] if isinstance(column, tuple) else column


def _column_label(column: Column) -> str:
    if isinstance(column, tuple):
        return column[1]
    return column


def render_table(items: Iterable[Mapping[str, object]], columns: Sequence[Column]) -> str:
    keys = [_column_key(column) for column in columns]
    labels = [_column_label(column) for column in columns]
    rows: list[list[str]] = []
    for item in items:
        rows.append([_stringify(item.get(key, "")) for key in keys])

    widths = [len(label) for label in labels]
    for row in rows:
        for idx, cell in enumerate(row):
            widths[idx] = max(widths[idx], len(cell))
    header = " ".join(labels[idx].ljust(widths[idx]) for idx in range(len(labels)))
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
