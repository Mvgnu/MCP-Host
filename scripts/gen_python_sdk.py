import argparse
import json
import sys
from urllib.request import urlopen, Request

TEMPLATE_HEADER = '''class MCPClient:
    def __init__(self, invoke_url: str, api_key: str):
        self.invoke_url = invoke_url.rstrip('/')
        self.headers = {'Authorization': f'Bearer {api_key}', 'Content-Type': 'application/json'}

    def _call(self, payload: dict):
        data = json.dumps(payload).encode()
        req = Request(self.invoke_url, data=data, headers=self.headers)
        with urlopen(req) as resp:
            return json.loads(resp.read())
'''


def fetch_config(host: str, server_id: str) -> dict:
    url = f"{host.rstrip('/')}/api/servers/{server_id}/client-config"
    with urlopen(url) as resp:
        return json.loads(resp.read())


def sanitize(name: str) -> str:
    return name.lower().replace(' ', '_')


def generate_sdk(cfg: dict) -> str:
    manifest = cfg.get('manifest', {})
    lines = [TEMPLATE_HEADER]
    caps = manifest.get('capabilities', [])
    for cap in caps:
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


def main():
    p = argparse.ArgumentParser(description='Generate Python SDK stub from MCP manifest')
    p.add_argument('server_id', help='Server ID to fetch configuration for')
    p.add_argument('--host', default='http://localhost:3000', help='Base URL of MCP Host')
    p.add_argument('--output', default='mcp_client.py', help='Destination file name')
    args = p.parse_args()

    try:
        cfg = fetch_config(args.host, args.server_id)
    except Exception as e:
        print(f'Error fetching configuration: {e}', file=sys.stderr)
        sys.exit(1)

    sdk_code = generate_sdk(cfg)
    with open(args.output, 'w') as f:
        f.write(sdk_code)
    print(f'SDK written to {args.output}')


if __name__ == '__main__':
    main()
