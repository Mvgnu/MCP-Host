import argparse
import json
import sys
from urllib.request import urlopen, Request


def fetch_config(base_url: str, server_id: str) -> dict:
    url = f"{base_url}/api/servers/{server_id}/client-config"
    req = Request(url)
    with urlopen(req) as resp:
        data = resp.read()
    return json.loads(data)


def main():
    parser = argparse.ArgumentParser(description="Fetch MCP client configuration")
    parser.add_argument("server_id", help="Server ID to fetch the config for")
    parser.add_argument("--host", default="http://localhost:3000", help="Base URL of MCP Host")
    parser.add_argument("--output", help="Path to save the config JSON")
    args = parser.parse_args()

    try:
        cfg = fetch_config(args.host.rstrip("/"), args.server_id)
    except Exception as e:
        print(f"Error fetching configuration: {e}", file=sys.stderr)
        sys.exit(1)

    if args.output:
        with open(args.output, "w") as f:
            json.dump(cfg, f, indent=2)
        print(f"Configuration saved to {args.output}")
    else:
        print(json.dumps(cfg, indent=2))


if __name__ == "__main__":
    main()
