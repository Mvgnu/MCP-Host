import argparse
import json
import sys
from pathlib import Path
from urllib.request import urlopen, Request

import requests
from fastapi import FastAPI
import uvicorn


def fetch_config(host: str, server_id: str) -> dict:
    url = f"{host.rstrip('/')}/api/servers/{server_id}/client-config"
    with urlopen(url) as resp:
        return json.loads(resp.read())


def sanitize(name: str) -> str:
    return name.lower().replace(' ', '_')


def generate_python_sdk(cfg: dict) -> str:
    manifest = cfg.get('manifest', {})
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
    for cap in manifest.get('capabilities', []):
        cname = cap.get('name')
        if not cname:
            continue
        method = sanitize(cname)
        desc = cap.get('description', '')
        lines.append(f"    def {method}(self, payload: dict) -> dict:")
        if desc:
            lines.append(f"        \"\"\"{desc}\"\"\"")
        lines.append(f"        return self._call({{'capability': '{cname}', 'input': payload}})")
        lines.append('')
    return '\n'.join(lines)


def generate_ts_sdk(cfg: dict) -> str:
    manifest = cfg.get('manifest', {})
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
    for cap in manifest.get('capabilities', []):
        cname = cap.get('name')
        if not cname:
            continue
        method = sanitize(cname)
        desc = cap.get('description', '')
        lines.append(f"    async {method}(payload: any): Promise<any> {{")
        if desc:
            lines.append(f"        // {desc}")
        lines.append(f"        return this.call({{'capability': '{cname}', 'input': payload}});")
        lines.append("    }")
        lines.append("")
    lines.append("}")
    return '\n'.join(lines)


TEMPLATE_FASTAPI = """from fastapi import FastAPI\nfrom mcp_client import MCPClient\n\nclient = MCPClient(\"{invoke_url}\", \"{api_key}\")\napp = FastAPI()\n\n@app.post('/invoke')\nasync def invoke(payload: dict):\n    return client._call(payload)\n"""

def cmd_create(args):
    if args.template != 'python-fastapi':
        raise SystemExit('only python-fastapi template is supported for now')
    cfg = fetch_config(args.host, args.mcp_id)
    proj = Path(args.name)
    proj.mkdir(parents=True, exist_ok=False)
    (proj / 'requirements.txt').write_text('fastapi\nuvicorn\nrequests\n')
    sdk_code = generate_python_sdk(cfg)
    (proj / 'mcp_client.py').write_text(sdk_code)
    main_code = TEMPLATE_FASTAPI.format(invoke_url=cfg['invoke_url'], api_key=cfg['api_key'])
    (proj / 'main.py').write_text(main_code)
    print(f'Scaffold created in {proj}')


def cmd_fetch(args):
    cfg = fetch_config(args.host, args.server_id)
    if args.output:
        with open(args.output, 'w') as f:
            json.dump(cfg, f, indent=2)
        print(f"Configuration written to {args.output}")
    else:
        print(json.dumps(cfg, indent=2))


def cmd_py(args):
    cfg = fetch_config(args.host, args.server_id)
    code = generate_python_sdk(cfg)
    with open(args.output, 'w') as f:
        f.write(code)
    print(f"Python SDK written to {args.output}")


def cmd_ts(args):
    cfg = fetch_config(args.host, args.server_id)
    code = generate_ts_sdk(cfg)
    with open(args.output, 'w') as f:
        f.write(code)
    print(f"TypeScript SDK written to {args.output}")


def cmd_dev(args):
    cfg = fetch_config(args.host, args.server_id)

    app = FastAPI()

    @app.post("/invoke")
    async def invoke(payload: dict):
        resp = requests.post(
            cfg["invoke_url"].rstrip("/"),
            json=payload,
            headers={"Authorization": f"Bearer {cfg['api_key']}", "Content-Type": "application/json"},
        )
        resp.raise_for_status()
        return resp.json()

    uvicorn.run(app, host="0.0.0.0", port=args.port)


def main():
    p = argparse.ArgumentParser(description='MCP Host helper CLI')
    sub = p.add_subparsers(dest='cmd', required=True)

    f = sub.add_parser('fetch-config', help='Fetch client configuration')
    f.add_argument('server_id')
    f.add_argument('--host', default='http://localhost:3000')
    f.add_argument('--output')
    f.set_defaults(func=cmd_fetch)

    py = sub.add_parser('gen-python', help='Generate Python SDK')
    py.add_argument('server_id')
    py.add_argument('--host', default='http://localhost:3000')
    py.add_argument('--output', default='mcp_client.py')
    py.set_defaults(func=cmd_py)

    ts = sub.add_parser('gen-ts', help='Generate TypeScript SDK')
    ts.add_argument('server_id')
    ts.add_argument('--host', default='http://localhost:3000')
    ts.add_argument('--output', default='mcp_client.ts')
    ts.set_defaults(func=cmd_ts)

    create = sub.add_parser('create', help='Scaffold an agent project')
    create.add_argument('name')
    create.add_argument('--mcp-id', required=True)
    create.add_argument('--template', default='python-fastapi')
    create.add_argument('--host', default='http://localhost:3000')
    create.set_defaults(func=cmd_create)

    dev = sub.add_parser('dev', help='Run a local proxy to the given server')
    dev.add_argument('server_id')
    dev.add_argument('--host', default='http://localhost:3000')
    dev.add_argument('--port', type=int, default=8000)
    dev.set_defaults(func=cmd_dev)

    args = p.parse_args()
    try:
        args.func(args)
    except Exception as e:
        print(f'Error: {e}', file=sys.stderr)
        sys.exit(1)


if __name__ == '__main__':
    main()
