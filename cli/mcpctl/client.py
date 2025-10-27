"""HTTP client helpers for the mission-control CLI."""
# key: operator-cli -> api-client

from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any, Dict, Iterable, Iterator, Mapping, MutableMapping, Optional

import requests
from requests import Response, Session

DEFAULT_TIMEOUT = 30


class APIError(RuntimeError):
    """Represents a non-success response from the MCP Host API."""

    def __init__(self, status_code: int, message: str, *, payload: Optional[Any] = None) -> None:
        super().__init__(f"{status_code}: {message}")
        self.status_code = status_code
        self.payload = payload


@dataclass(slots=True)
class APIClient:
    """Small wrapper around :mod:`requests` with auth + pagination helpers."""

    base_url: str
    token: Optional[str] = None
    timeout: int = DEFAULT_TIMEOUT
    session: Optional[Session] = None

    def __post_init__(self) -> None:
        if self.session is None:
            self.session = requests.Session()

    # key: http-request -> auth-header
    def _build_headers(self, headers: Optional[Mapping[str, str]]) -> Dict[str, str]:
        merged: Dict[str, str] = {"Accept": "application/json"}
        if headers:
            merged.update(headers)
        if self.token:
            merged.setdefault("Authorization", f"Bearer {self.token}")
        return merged

    def request(
        self,
        method: str,
        path: str,
        *,
        params: Optional[Mapping[str, Any]] = None,
        json_body: Optional[Any] = None,
        headers: Optional[Mapping[str, str]] = None,
    ) -> Any:
        url = self._join(path)
        response = self.session.request(
            method,
            url,
            params=params,
            json=json_body,
            timeout=self.timeout,
            headers=self._build_headers(headers),
        )
        return self._handle_response(response)

    def get(self, path: str, *, params: Optional[Mapping[str, Any]] = None) -> Any:
        return self.request("GET", path, params=params)

    def post(
        self,
        path: str,
        *,
        json_body: Optional[Any] = None,
        params: Optional[Mapping[str, Any]] = None,
    ) -> Any:
        return self.request("POST", path, json_body=json_body, params=params)

    def patch(
        self,
        path: str,
        *,
        json_body: Optional[Any] = None,
        params: Optional[Mapping[str, Any]] = None,
    ) -> Any:
        return self.request("PATCH", path, json_body=json_body, params=params)

    def delete(self, path: str, *, params: Optional[Mapping[str, Any]] = None) -> Any:
        return self.request("DELETE", path, params=params)

    def paginate(
        self,
        path: str,
        *,
        params: Optional[MutableMapping[str, Any]] = None,
        data_key: str = "items",
    ) -> Iterator[Any]:
        """Iterate through paginated responses using ``next`` links or offsets."""

        params = dict(params or {})
        next_url: Optional[str] = None
        while True:
            payload = self.request("GET", next_url or path, params=params if next_url is None else None)
            if isinstance(payload, Mapping):
                if data_key in payload and isinstance(payload[data_key], Iterable):
                    for item in payload[data_key]:
                        yield item
                elif data_key == "items":
                    # Accept bare arrays for convenience.
                    items = payload if isinstance(payload, list) else payload.get("data")
                    if isinstance(items, Iterable):
                        for item in items:
                            yield item
                next_url = payload.get("next") if isinstance(payload, Mapping) else None
                if next_url:
                    continue
                if "offset" in params and "total" in payload:
                    offset = params.get("offset", 0)
                    limit = params.get("limit")
                    total = payload.get("total")
                    if isinstance(limit, int) and isinstance(offset, int) and isinstance(total, int):
                        offset += limit
                        if offset >= total:
                            break
                        params["offset"] = offset
                        continue
            break

    def _join(self, path: str) -> str:
        if path.startswith("http://") or path.startswith("https://"):
            return path
        return f"{self.base_url.rstrip('/')}/{path.lstrip('/')}"

    def _handle_response(self, response: Response) -> Any:
        if 200 <= response.status_code < 300:
            if response.content:
                try:
                    return response.json()
                except json.JSONDecodeError:
                    return response.text
            return None
        try:
            payload = response.json()
        except json.JSONDecodeError:
            payload = response.text
        message = payload if isinstance(payload, str) else payload.get("error") or json.dumps(payload)
        raise APIError(response.status_code, message, payload=payload)

    def stream_sse(
        self,
        path: str,
        *,
        params: Optional[Mapping[str, Any]] = None,
    ) -> Iterator[str]:
        url = self._join(path)
        headers = self._build_headers({"Accept": "text/event-stream"})
        with self.session.get(
            url,
            params=params,
            headers=headers,
            timeout=self.timeout,
            stream=True,
        ) as response:
            if not 200 <= response.status_code < 300:
                try:
                    payload = response.json()
                except json.JSONDecodeError:
                    payload = response.text
                message = (
                    payload
                    if isinstance(payload, str)
                    else payload.get("error") or json.dumps(payload)
                )
                raise APIError(response.status_code, message, payload=payload)

            for raw_line in response.iter_lines(decode_unicode=True):
                if raw_line is None:
                    continue
                line = raw_line.strip()
                if not line or line.startswith(":"):
                    continue
                if line.startswith("data:"):
                    yield line[5:].strip()
