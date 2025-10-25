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
