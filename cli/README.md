# MCP Operator Mission-Control CLI

The `mcpctl` tool provides a terminal-first workflow for marketplace inspection, promotion lifecycle management, governance orchestration, and evaluation maintenance. It replaces the former `scripts/mcp_cli.py` helper with a modular package that maps one-to-one onto the backend API surface.

## Installation

```bash
pip install -e ./cli
```

The editable install exposes the `mcpctl` console entry point. Authentication defaults to the `MCP_HOST_TOKEN` environment variable and the host URL defaults to `MCP_HOST_URL` (falling back to `http://localhost:3000`).

## Command groups

### Marketplace

```bash
mcpctl marketplace list
```

Lists marketplace artifacts and their active status. Pass `--json` to receive raw JSON responses.

### Promotions

* `mcpctl promotions tracks list` – enumerate the promotion tracks available to the authenticated operator.
* `mcpctl promotions history [--track-id TRACK] [--manifest DIGEST]` – inspect promotion history.
* `mcpctl promotions schedule TRACK_ID MANIFEST STAGE [--artifact-run-id RUN] [--note NOTE]` – schedule a promotion.
* `mcpctl promotions approve PROMOTION_ID [--note NOTE]` – approve a scheduled promotion.

### Governance

* `mcpctl governance workflows list` – view configured governance workflows.
* `mcpctl governance workflows start WORKFLOW_ID [--artifact-run-id RUN] [--manifest-digest DIGEST] [--context JSON]` – trigger a workflow run.
* `mcpctl governance runs get RUN_ID` – inspect a workflow run’s status.

### Evaluations

* `mcpctl evaluations list` – list evaluation certifications across artifacts.
* `mcpctl evaluations retry EVALUATION_ID` – schedule a certification retry.

### Scaffolding helpers

The legacy scaffolding commands remain available under the `scaffold` group:

* `mcpctl scaffold fetch-config SERVER_ID [--json]`
* `mcpctl scaffold gen-python SERVER_ID [--output PATH]`
* `mcpctl scaffold gen-ts SERVER_ID [--output PATH]`
* `mcpctl scaffold create NAME --server-id SERVER_ID`

## Output formats

Every subcommand accepts `--json`. When omitted, the CLI renders simple tables that emphasize workflow status and tier information.

## Testing

Run the integration-style CLI tests with:

```bash
pytest cli/tests/test_cli.py
```

The tests stub the HTTP client to validate serialization, command wiring, and argument parsing without requiring a live backend.
