"""Agent scaffolding helpers for the mission-control CLI."""
# key: operator-cli -> scaffold

from __future__ import annotations

import json
from pathlib import Path
from typing import Dict
from urllib.request import Request, urlopen


def sanitize(name: str) -> str:
    return name.lower().replace(" ", "_")


def generate_python_sdk(cfg: Dict[str, object]) -> str:
    manifest = cfg.get("manifest", {}) if isinstance(cfg, dict) else {}
    lines = [
        "class MCPClient:",
        "    def __init__(self, invoke_url: str, api_key: str):",
        "        self.invoke_url = invoke_url.rstrip('/')",
        "        self.headers = {'Authorization': f'Bearer {api_key}', 'Content-Type': 'application/json'}",
        "",
        "    def _call(self, payload: dict):",
        "        data = json.dumps(payload).encode()",
        "        req = Request(self.invoke_url, data=data, headers=self.headers)",
        "        with urlopen(req) as resp:",
        "            return json.loads(resp.read())",
        "",
    ]
    for capability in manifest.get("capabilities", []) if isinstance(manifest, dict) else []:
        name = capability.get("name")
        if not name:
            continue
        method = sanitize(name)
        description = capability.get("description", "")
        lines.append(f"    def {method}(self, payload: dict) -> dict:")
        if description:
            lines.append(f"        \"\"\"{description}\"\"\"")
        lines.append("        return self._call({'capability': '%s', 'input': payload})" % name)
        lines.append("")
    return "\n".join(lines)


def generate_ts_sdk(cfg: Dict[str, object]) -> str:
    manifest = cfg.get("manifest", {}) if isinstance(cfg, dict) else {}
    lines = [
        "export class MCPClient {",
        "    constructor(public invokeUrl: string, public apiKey: string) {}",
        "",
        "    private async call(payload: any): Promise<any> {",
        "        const res = await fetch(this.invokeUrl.replace(/\\/$/, ''), {",
        "            method: 'POST',",
        "            headers: {",
        "                'Authorization': `Bearer ${this.apiKey}`,",
        "                'Content-Type': 'application/json'",
        "            },",
        "            body: JSON.stringify(payload)",
        "        });",
        "        if (!res.ok) {",
        "            throw new Error(`Request failed with ${res.status}`);",
        "        }",
        "        return res.json();",
        "    }",
        "",
    ]
    for capability in manifest.get("capabilities", []) if isinstance(manifest, dict) else []:
        name = capability.get("name")
        if not name:
            continue
        method = sanitize(name)
        description = capability.get("description", "")
        lines.append(f"    async {method}(payload: any): Promise<any> {{")
        if description:
            lines.append(f"        // {description}")
        lines.append("        return this.call({'capability': '%s', 'input': payload});" % name)
        lines.append("    }")
        lines.append("")
    lines.append("}")
    return "\n".join(lines)


TEMPLATE_FASTAPI = """from fastapi import FastAPI\nfrom mcp_client import MCPClient\n\nclient = MCPClient(\"{invoke_url}\", \"{api_key}\")\napp = FastAPI()\n\n@app.post('/invoke')\nasync def invoke(payload: dict):\n    return client._call(payload)\n"""


def write_fastapi_project(target: Path, cfg: Dict[str, object]) -> None:
    target.mkdir(parents=True, exist_ok=False)
    (target / "requirements.txt").write_text("fastapi\nuvicorn\nrequests\n", encoding="utf-8")
    (target / "mcp_client.py").write_text(generate_python_sdk(cfg), encoding="utf-8")
    invoke_url = cfg.get("invoke_url", "") if isinstance(cfg, dict) else ""
    api_key = cfg.get("api_key", "") if isinstance(cfg, dict) else ""
    main_code = TEMPLATE_FASTAPI.format(invoke_url=invoke_url, api_key=api_key)
    (target / "main.py").write_text(main_code, encoding="utf-8")
