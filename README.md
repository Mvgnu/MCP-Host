# MCP Host

A Model Context Protocol hosting platform.

## Backend configuration

The backend exposes several environment variables to control startup behavior:

| Variable | Description | Default |
| --- | --- | --- |
| `BIND_ADDRESS` | Address the HTTP server listens on. | `0.0.0.0` |
| `BIND_PORT` | Port the HTTP server listens on. | `3000` |
| `ALLOW_MIGRATION_FAILURE` | When set to `true`, allows boot to continue even if database migrations fail. | `false` |

Set these variables in your deployment environment (or a local `.env` file) to adjust how the API service starts.

### Registry push operations

The build service tags and pushes images using the Docker remote API via Bollard. Key behaviors:

* Registry endpoints are emitted via `tracing` with target `registry.push`, including the derived scopes (`repository:<image>:push/pull`) and server ID.
* Progress logs include digest discovery lines such as `Manifest published with digest sha256:<hash>` that propagate to the UI.
* Authentication failures generate `registry authentication expired` errors and surface to the server status feed so operators can refresh credentials.
* Transient transport errors (I/O, hyper, HTTP client, or timeouts) retry up to `REGISTRY_PUSH_RETRIES` attempts (default `3`) with a short backoff. Override the limit via an environment variable when tuning resilience.
* Usage metrics capture each stage: `tag_started`/`tag_succeeded` for Docker tagging and `push_failed` entries with `attempt=0` for pre-stream failures, giving observability platforms enough context to differentiate tagging issues from push retries.

#### Runbook

1. **Verify telemetry** – search your log aggregator for `target="registry.push" registry push failed` events to identify the failing repository and scope.
2. **Check digest messages** – if `Manifest published` entries are missing, confirm the registry user has push permissions for the derived scopes.
3. **Handle auth expiry** – when the error includes `authentication required`, refresh the registry credentials and redeploy. The build will emit an explicit `registry authentication expired` message.
4. **Transient faults** – for recurring network hiccups, increase `REGISTRY_PUSH_RETRIES` temporarily and monitor retry success events (`registry push succeeded after retry`).
5. **Status recovery** – failed pushes mark the server `error`; once the issue is resolved trigger a redeploy to rebuild and push a fresh image.
