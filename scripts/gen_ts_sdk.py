import argparse
import json
import sys
from urllib.request import urlopen, Request

TEMPLATE_HEADER = """export class MCPClient {
    constructor(public invokeUrl: string, public apiKey: string) {}

    private async call(payload: any): Promise<any> {
        const res = await fetch(this.invokeUrl.replace(/\/$/, ''), {
            method: 'POST',
            headers: {
                'Authorization': `Bearer ${this.apiKey}`,
                'Content-Type': 'application/json'
            },
            body: JSON.stringify(payload)
        });
        if (!res.ok) {
            throw new Error(`Request failed with ${res.status}`);
        }
        return res.json();
    }
"""

TEMPLATE_FOOTER = """}
"""

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
        lines.append(f"    async {method}(payload: any): Promise<any> {{")
        if desc:
            lines.append(f"        // {desc}")
        lines.append(f"        return this.call({{'capability': '{cname}', 'input': payload}});")
        lines.append("    }")
        lines.append("")
    lines.append(TEMPLATE_FOOTER)
    return '\n'.join(lines)

def main():
    p = argparse.ArgumentParser(description='Generate TypeScript SDK stub from MCP manifest')
    p.add_argument('server_id', help='Server ID to fetch configuration for')
    p.add_argument('--host', default='http://localhost:3000', help='Base URL of MCP Host')
    p.add_argument('--output', default='mcp_client.ts', help='Destination file name')
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
