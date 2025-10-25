# Progress Log

This document tracks development progress and high level notes from the planning materials in `README.md`.

## 2024-05-04
- Created `backend` Rust project using `cargo init`.
- Added dependencies (`axum`, `tokio`, `serde`, `sqlx`, etc.) and basic HTTP server at `/` returning placeholder text.
- Generated `frontend` Next.js project with TypeScript and Tailwind CSS using `create-next-app`.
- Added repository level `.gitignore` to exclude build artifacts and environment files.
- Setup progress log.

### Next Steps
- Implement initial database migrations using `sqlx` or Diesel.
- Set up authentication endpoints (register/login) in the backend.
- Flesh out frontend pages for login and registration.

## 2025-07-02
- Added SQL migration file defining initial PostgreSQL schema.
- Implemented database connection pool and migration execution on startup.
- Added user registration and login endpoints with password hashing and JWTs.
- Introduced `AuthUser` extractor for protected routes.
- Updated Cargo dependencies with chrono and async-trait.

### Next Steps
- Add endpoints for MCP server management and container launch.
- Create frontend pages for registration and login forms.

## 2025-07-03
- Implemented basic MCP server management API:
  - `GET /api/servers` lists servers for the authenticated user.
  - `POST /api/servers` creates a new server entry with a generated API key.
  - `POST /api/servers/:id/start` marks a server as running (placeholder for Docker launch).
- Added new `servers` module and integrated routes in `main.rs`.
- Extended Cargo dependencies with `bollard` for future Docker integration.

### Next Steps
- Flesh out Docker container management using `bollard`.
- Add frontend pages to view and create MCP servers.

## 2025-07-04
- Implemented asynchronous Docker container launch using `bollard`:
  - Added `docker.rs` with `spawn_server_task` to create and start containers.
  - Updated `start_server` handler to invoke this task and transition status.
  - Enhanced `create_server` to return server info and immediately launch the container.
- Extended `sqlx` features to support `chrono` types.
- Added simple Next.js pages to list existing servers and create new ones.

### Next Steps
- Wire frontend pages into navigation.
- Add error handling and container stop/delete endpoints.

## 2025-07-05
- Added stop and delete functionality for MCP servers:
  - New `stop_server_task` and `delete_server_task` in `docker.rs` to control containers.
  - API routes `/api/servers/:id/stop` and `DELETE /api/servers/:id` wired into Axum.
- Enhanced frontend with a navigation bar linking to all pages.
- Installed `rustfmt` for consistent formatting.

### Next Steps
- Surface container status updates in the frontend.
- Improve form error handling and authentication flow.

## 2025-07-06
- Surfaced container status on the frontend with a polling server list and start/stop/delete actions.
- Added basic error handling and redirects for login, registration, and new server forms.
- Updated pages to display API error messages.

### Next Steps
- Persist user session state in the frontend.
- Display logs or detailed status for each server.

## 2025-07-07
- Added `/api/me` and `/api/logout` endpoints for retrieving current user info and clearing the auth cookie.
- Implemented `/api/servers/:id/logs` to fetch recent container logs via Docker.
- Created a React session provider to load the current user and expose login state across the app.
- Updated navigation to show logout with the user's email when authenticated.
- Servers page now offers a "Logs" button to display container output.

### Next Steps
- Persist logs to the database for historical viewing.
- Polish frontend styling and add loading indicators.
## 2025-07-08
- Added `server_logs` table and migration to persist container logs.
- Logs endpoint now saves output to the database.
- New `/api/servers/:id/logs/history` returns recent stored logs.

### Next Steps
- Improve frontend to show log history and live updates.
- Enhance UI polish and add loading indicators.
## 2025-07-09
- Added ability to specify a custom Docker image when creating a server ("bring your own MCP")
- Updated Docker helper to read `config.image` for Custom servers
- Extended new server form to support the custom image option
- Server list now fetches stored log history and displays timestamps

### Next Steps
- Stream live logs via SSE for real-time updates
- Add loading indicators and improved styling

## 2025-07-10
- Implemented SSE endpoint `/api/servers/:id/logs/stream` for live container logs.
- Docker module spawns log streaming tasks that persist output.
- Frontend connects via EventSource to show real-time logs with a close option.

### Next Steps
- Polish UI styling and add loading indicators.
## 2025-07-11
- Added Spinner component for loading indicators.
- Updated login, registration, and new server forms to disable submit buttons and show spinner during requests.
- Enhanced servers page with loading state, action spinners on control buttons, and a top-level spinner while fetching data.
- Documented these UI improvements.

### Next Steps
- Further refine layout and error messaging.


## 2025-07-12
- Created reusable `Alert` component for displaying success and error messages.
- Added page headings and improved form layout across login, registration and new server pages.
- Enhanced servers page with better error handling for start/stop/delete actions and cleaner log viewing controls.

### Next Steps
- Finish UI polish and begin integrating metrics collection.

## 2025-07-13
- Added usage metrics endpoints to backend and recorded start/stop/delete events
- Frontend can now view metrics per server
- Minor layout tweaks and centered main content

### Next Steps
- Stream metrics live via SSE and display charts

## 2025-07-14
- Implemented broadcast channels for metrics and new `/api/servers/:id/metrics/stream` endpoint
- Frontend now opens an EventSource to receive live metrics updates
- Documented changes and prepared for chart visualization

### Next Steps
- Visualize metrics with simple charts

## 2025-07-15
- Added `react-chartjs-2` and `chart.js` to the frontend for metrics visualization.
- Created `MetricsChart` component rendering a line chart of events.
- Updated servers page to display charts when viewing metrics with live SSE updates.

### Next Steps
- Refine chart styles and allow filtering by event type.

## 2025-07-16
- Polished metrics charts with a darker theme and legend at the bottom.
- Added filter checkboxes so users can toggle event types on and off.
- MetricsChart now shows separate lines for each event type using distinct colors.

### Next Steps
- Expand documentation pages and continue refining the frontend layout.

## 2025-07-17
- Created reusable `Button`, `Hero`, and `Section` components for a more expressive UI.
- Added Docs and Blog pages describing MCP Host features and placeholder articles.
- Revised the landing page to use the new components with clear calls to action.
- Navigation bar now links to Docs and Blog.
- Updated frontend dependencies to include `clsx` for styling utilities.

### Next Steps
- Flesh out documentation content and expand blog posts.
- Continue polishing components and responsive design.

## 2025-07-18
- Extended server creation form with support for environment variable pairs.
- Docker helper now ignores the `image` field when injecting config values as
  `CFG_` environment variables.
- Documentation updated with details on passing custom configuration via the UI.

### Next Steps
- Expand the BYO MCP guide with more examples.
- Polish dashboard layout and styling.

## 2025-07-19
- Expanded the documentation with a richer BYO MCP guide including sample
  environment variables and image hints.
- Refined the servers dashboard with card-style layout and rounded action
  buttons for a cleaner look.

### Next Steps
- Continue improving overall styling and add more blog content.

## 2025-07-20
- Introduced prebuilt service integrations with a new `service_integrations` table.
- Backend exposes `/api/servers/:id/services` to list and create service attachments.
- Docker task now injects `REDIS_URL` or `S3_BUCKET` variables based on attached services.
- Added Next.js page to manage services per server and linked it from the dashboard.
- Documentation updated with a "Prebuilt Service Integrations" section.

### Next Steps
- Support editing and removing integrations.
- Explore automatic deployments from git repositories.

## 2025-07-21
- Added API endpoints to update and delete service integrations.
- Services page now lists each integration with Edit and Delete actions.
- Updated docs to mention editing and removing integrations.

### Next Steps
- Investigate git-based deployments for custom MCP servers.

## 2025-07-22
- Implemented experimental git-based deployment flow. `spawn_server_task` now
  clones a provided `repo_url`, builds a Docker image, and runs it for Custom
  servers.
- Server creation page accepts a Git repository URL in addition to a custom
  image name.
- Documentation updated to explain the new option under "Bring Your Own MCP".
- Added `tempfile` dependency to manage build workspaces.

### Next Steps
- Harden build logic and add progress feedback during image builds.

## 2025-07-23
- Improved git-based deployments with progress tracking:
  - `spawn_server_task` now updates status to `cloning` and `building` while preparing images
  - progress messages are saved to `server_logs` for streaming to the UI
  - container stop/delete tasks also log actions
- Docs note that build status and logs appear during BYO MCP deployments

### Next Steps
- Explore security around build contexts and caching

## 2025-07-24
- Secured git-based builds by using `docker build --pull --no-cache` and
  cleaning temporary directories after each build.
- Updated BYO MCP documentation with a note about fresh build flags and
  temporary workspaces.
- Logged progress messages like "Cleaning up" so users know when build
  directories are removed.

### Next Steps
- Investigate automatic deployment triggers from git pushes and support for
  branch selection.

## 2025-07-25
- Added optional `branch` setting for git-based deployments and updated the
  Docker task to clone the specified branch.
- New `redeploy_server` endpoint allows triggering a rebuild via API, removing
  any existing container before launching the new image.
- Server dashboard includes a Redeploy button and new server form lets users
  specify the repository branch.
- Documentation updated with branch and webhook notes for BYO MCP.

### Next Steps
- Test webhook-based redeploys and expand CI integration examples.

## 2025-07-26
- Added `webhook_secret` column and generated a secret when creating servers.
- New `/api/servers/:id/webhook` endpoint allows unauthenticated redeploys when the secret matches.
- Docs updated to show how to call the webhook endpoint.
- Logged progress toward automated CI triggers.

### Next Steps
- Integrate GitHub webhooks for automatic redeploys.

## 2025-07-27
- Implemented `/api/servers/:id/github` endpoint that verifies HMAC signatures
  from GitHub push webhooks and triggers a redeploy when valid.
- Added `hmac`, `sha2`, and `hex` dependencies for signature checks.
- Docs now explain configuring GitHub webhooks to hit this endpoint.
- Updated BYO MCP guide with a bullet about the new GitHub integration.

### Next Steps
- Test webhook delivery end-to-end and continue refining the UI.

## 2025-07-28
- Introduced `custom_domains` table to map external domains to MCP servers.
- Added new backend module `domains` with list/create/delete endpoints.
- Router now exposes `/api/servers/:id/domains` and `/api/servers/:id/domains/:domain_id`.
- Created frontend management page for custom domains and link from servers dashboard.
- Documentation mentions custom domains for BYO MCP deployments.

### Next Steps
- Integrate domain verification and automated HTTPS provisioning.
## 2025-07-29
- Introduced secret management:
  - New `server_secrets` table with encrypted values (pgcrypto)
  - API endpoints `/api/servers/:id/secrets` for CRUD operations
  - Docker tasks inject secrets as environment variables when launching containers
- Updated README with a bullet about secret management
- Noted next steps for proxy controller and automated HTTPS

## 2025-07-30
- Implemented simple `proxy` module generating Nginx configs per server
- Domain create/delete triggers proxy rebuild so custom URLs work immediately
- Docker tasks refresh proxy when servers start, stop, or are deleted
- README mentions the new reverse proxy controller

### Next Steps
- Automate TLS certificates and explore build pipeline for custom code

## 2025-07-31
- Added automatic TLS provisioning. `proxy` now runs `certbot` for each domain
  when proxy configs are rebuilt.
- Set `CERTBOT_EMAIL` environment variable so certificates can be issued.
- `.gitignore` now excludes generated `proxy_conf/` files.
- Documented automatic TLS in README under infrastructure bullets.

### Next Steps
- Prototype build orchestrator for Dockerfile parsing and custom language support.

## 2025-08-01
- Created a new `build` module with `build_from_git` helper that clones a
  repository, parses its Dockerfile for exposed ports, and builds the image.
- `docker` module now delegates Git builds to this helper so custom MCP servers
  are assembled consistently.
- README updated to mention source builds and the docs detail Dockerfile
  parsing warnings.

### Next Steps
- Expand the build pipeline with language-specific builders and push images to a registry.

## 2025-08-02
- Added automatic Dockerfile generation when building from git sources.
- Build orchestrator detects Node, Python, or Rust projects and creates a simple Dockerfile if none exists.
- Images are optionally pushed to a registry when the `REGISTRY` env var is set.
- Documentation updated with details on the language builders and registry support.

### Next Steps
- Polish BYO MCP instructions and focus on plug-and-play usage of custom servers.


## 2025-08-03
- Introduced basic file storage module with new `server_files` table.
- Added API endpoints for listing, uploading, downloading, and deleting files.
- Backend stores files under `storage/<server_id>/` and records metadata in the database.
- Updated `.gitignore` to exclude the storage directory.
- Documented file storage API in the README.

### Next Steps
- Surface file uploads in the frontend and allow MCP servers to persist artifacts.

## 2025-08-04
- Added Files page in the frontend to upload, download and delete persistent blobs per server.
- Servers list now links to this new page for easy access.
- Updated README bullets to mention BYO custom images and the file management UI.

### Next Steps
- Mount uploaded files into running containers so MCP servers can read and write data.

## 2025-08-05
- Mounted each server's `storage/<id>` directory into its container at `/data` so uploads are accessible at runtime.
- Storage directories are created automatically and removed when servers are deleted.
- README notes that uploaded files appear inside containers under `/data`.

### Next Steps
- Explore GPU inference support and dynamic scaling options.

## 2025-08-06
- Added GPU support: servers can request Nvidia GPUs via a new `use_gpu` flag.
- Updated Docker launcher to pass `device_requests` when GPUs are enabled.
- Frontend form includes a GPU checkbox and the servers list shows a GPU badge.
- README and docs mention GPU-enabled deployments.

### Next Steps
- Investigate auto-scaling policies and runtime resource limits.

## 2025-08-07
- Implemented automatic container monitoring and restart logic.
- New `monitor_server_task` watches each container and triggers a rebuild if it
  exits unexpectedly, recording a `restart` metric.
- Documentation now notes crash restarts and docs page mentions the feature.

### Next Steps
- Explore more advanced scaling strategies and resource limits.

## 2025-08-08
- Added `/api/servers/:id/invoke` endpoint to proxy JSON requests to running MCP containers.
- Created Invoke page in the dashboard so users can test their deployments.
- Updated docs and README with instructions for the new invoke functionality.

### Next Steps
- Polish MCP interaction flows and continue improving the BYO workflow.

## 2025-08-09
- Added `manifest` column and migration to store MCP metadata.
- Containers fetch `/.well-known/mcp.json` after start and save it for clients.
- New `/api/servers/:id/manifest` route and dashboard page show the stored manifest.
- Documentation lists the manifest handshake for plug-and-play MCPs.

### Next Steps
- Experiment with automatic agent configuration using saved manifests.

## 2025-08-10
- Created `server_capabilities` table and migration.
- Container startup now parses `capabilities` from the MCP manifest and saves them.
- Exposed `/api/servers/:id/capabilities` endpoint and dashboard page.
- Documentation updated describing automatic capability sync.

### Next Steps
- Investigate using saved capabilities for auto-generated client configs.

## 2025-08-11
- Added `/api/servers/:id/client-config` endpoint returning invoke URL, API key,
  and stored manifest so agents can connect with zero setup.
- Documentation updated describing the new client configuration endpoint.

### Next Steps
- Prototype tooling that consumes this endpoint to generate ready-made SDK configs.
## 2025-08-12
- Added a `get_config.py` helper script under `scripts/`.
- The script fetches `/api/servers/:id/client-config` and writes the response to a JSON file for easy SDK setup.
- README notes how to use the script so agents can retrieve invoke URLs and API keys automatically.

### Next Steps
- Experiment with generating language-specific SDK stubs from the saved manifest.
## 2025-08-13
- Introduced gen_python_sdk.py script to generate a Python client from the stored manifest.
- README documents how to use the script with the client-config endpoint.
### Next Steps
- Extend the generator to output TypeScript or other languages.
## 2025-08-14
- Added gen_ts_sdk.py script to generate a TypeScript client from the stored MCP manifest.
- README documents using the new script alongside the Python generator for plug-and-play SDKs.

### Next Steps
- Explore packaging these generators into a CLI tool for easier distribution.

## 2025-08-15
- Added `mcp_cli.py` which consolidates config fetching and SDK generation into one command line tool.
- README documents using the CLI for quick plug-and-play MCP client setup.

### Next Steps
- Package the CLI and SDK generators for distribution via PyPI and npm.
## 2025-08-16
- Created `improvements.md` to track technical debt and planned fixes.
- Implemented secure webhook authentication via `X-Webhook-Secret` header.
- Added migration `0011_add_indexes.sql` to create indexes on foreign keys.

### Next Steps
- Refactor error handling to log underlying errors via `tracing` macros.
- Begin adding unit tests for authentication utilities.

## 2025-08-17
- Improved error handling across backend modules to log underlying errors using `tracing::error!`.
- Updated improvements tracker to mark logging task complete.

### Next Steps
- Begin adding unit tests for authentication utilities.
## 2025-08-18
- Added basic backend unit tests for AuthUser extractor and frontend Jest test for Button component.
- Replaced several `unwrap` calls with `expect` or proper error handling.
- Updated server metrics SSE to log serialization errors.
- Marked testing task complete in improvement tracker.


## 2025-07-03
- Replaced git and docker shell commands with git2 and bollard build_image in build helper.
- Updated dependencies and marked item complete in improvements tracker.

## 2025-07-03
- Introduced config module requiring JWT_SECRET at startup.
- Replaced runtime environment lookups with static secret reference.
- Removed unused imports and structs causing warnings.
- Updated README with note about mandatory JWT_SECRET.


## 2025-08-19
- Replaced certbot and nginx shell calls with internal ACME client and signal-based reload.
- Updated proxy module to use acme2 and nix crates.
- README documents new embedded TLS provisioning.
- Marked proxy improvement complete.

## 2025-08-20
- Introduced Zustand store and SSE stream for server status
- Added ServerCard component to simplify server list
- Backend broadcasts status changes over SSE
- Marked frontend state management tasks complete

## 2025-08-21
- Added in-memory job queue to decouple API from Docker tasks
- Server management routes now send jobs to worker thread
- Fixed Zustand import warning


## 2025-08-22
- Integrated optional HashiCorp Vault client for secret storage
- Secrets API stores paths in Vault when `VAULT_ADDR` and `VAULT_TOKEN` are set
- Docker helper fetches secrets from Vault at runtime
- Marked centralized secrets manager improvement complete

## 2025-08-23
- Added basic CI workflow running backend and frontend tests
- Added unit tests for the build helper and Zustand store
- Marked comprehensive test coverage item complete in improvement tracker

## 2025-08-24
- Added Python packaging in `cli/` so the helper CLI can be installed via `pip install .`
- README documents `mcp-cli` installation instructions
- Marked packaging task complete in improvements tracker

## 2025-08-25
- Created `refinement.md` outlining the next phases of work
- Began refactoring backend by moving API routes into a new module
## 2025-08-26\n- Added AppError with IntoResponse for consistent errors\n- Updated auth and servers modules to use AppResult\n- Introduced SWR-based useApi hook and refactored services page\n- Enhanced CI workflow with separate backend and frontend jobs\n
## 2025-08-27
- Isolated proxy functionality into separate `proxy_controller` binary that watches config directory and handles TLS and Nginx reloads
- Updated `proxy.rs` to only write configs
- Added new dependency `anyhow`

### 2025-08-28
- Enabled structured JSON logging with `tracing_subscriber` and environment filter
- Exposed Prometheus metrics via `/metrics`
- Added first integration test for the root route

## 2025-08-29\n- Added integration test for metrics endpoint\n- Introduced Playwright with a basic home page e2e test\n- CI workflow runs Playwright tests after installing browsers\n
## 2025-08-30
- Prepared CLI for PyPI distribution by expanding setup.py metadata
- README updated with installation instructions from PyPI


## 2025-08-31
- Introduced a `ContainerRuntime` trait with a `DockerRuntime` implementation
- Job worker and server handlers now use this trait, laying groundwork for future Kubernetes support
## 2025-09-01
- Added persistent job queue using database table and updated worker to replay queued jobs
- Server handlers now enqueue jobs in the database

## 2025-09-02
- Introduced simple RBAC with `role` column and per-user `server_quota`
- Admins can list all servers while regular users are limited to their own
- Server creation checks the quota and rejects when exceeded

## 2025-09-03
- Added `CONTAINER_RUNTIME` config to allow switching container backends
- Currently only Docker is implemented; selecting `kubernetes` logs a warning
- Documented the variable in the README

## 2025-09-04
- Introduced a stub `KubernetesRuntime` using the `kube` crate
- `main` now initializes this runtime when `CONTAINER_RUNTIME=kubernetes`,
  falling back to Docker on failure
- Updated README to document basic Kubernetes support
## 2025-09-05
- Implemented full Kubernetes runtime: pods are created for servers and logs streamed via API

## 2025-09-06
- Added `K8S_NAMESPACE` configuration so Kubernetes runtime can target custom namespaces
- Updated docs with the new variable and refactored runtime to use it

## 2025-09-07
- Added `K8S_SERVICE_ACCOUNT` configuration so pods use a specific service account
- Updated Kubernetes runtime to set `serviceAccountName`
- Documented the variable in the README

## 2025-09-08
- Containers now honor `cpu_limit` and `memory_limit` in server config for Docker and Kubernetes runtimes
- Documented the new limits in the README

## 2025-09-09
- Added regression test ensuring backend fails when JWT_SECRET is unset
## 2025-09-10
- Drafted vision for the "Context Cloud" with marketplace, managed vector DBs, ingestion pipelines, edge deployments and other advanced features
- Added these initiatives to refinement.md for future implementation
## 2025-09-11
- Introduced marketplace endpoint listing official MCP images for one-click deployment

## 2025-09-12
- Added `create` command to `mcp-cli` for scaffolding a Python FastAPI agent preconfigured with a selected MCP server
- Updated README with example usage of the new CLI command
## 2025-09-13
- Added "dev" command to mcp-cli for running a local proxy to an MCP server
- README documents using the dev command

## 2025-09-14
- Added managed vector database support with new vector_dbs table and Docker containers
- `/api/vector-dbs` endpoints create and delete Chroma instances
- README documents vector DB capability

## 2025-09-15
- Added data ingestion pipeline support with ingestion_jobs table and worker
- New /api/ingestion-jobs endpoints allow creating and deleting jobs

## 2025-09-16
- Added workflows feature allowing chaining servers together.
- New /api/workflows endpoints support creation, deletion, and invocation.

## 2025-09-17
- Added invocation tracing storing request and response pairs.
- New /api/servers/:id/invocations endpoint lists recent traces.


## 2025-09-18
- Added evaluation feature allowing tests to be created and run against servers.
- Results stored with similarity score using Jaro-Winkler metric.
## 2025-09-19
- Introduced organizations with membership roles.
- Added routes `/api/orgs` for creation and listing, and `/api/orgs/:id/members` for inviting users.
- Servers can optionally belong to an organization via `organization_id`.

## 2025-09-20
- Improved invocation logging with error handling when writes fail
- Completed Phase 1 tasks from refinement plan
## 2025-09-21
- Replaced header parsing unwraps with expect in auth module for robustness

\n## 2025-09-22\n- Added evaluation management page allowing tests to be created and run from the dashboard.\n- Button component supports disabled state and custom classes.\n- Progress logged for frontend integration of evaluation features.

## 2025-09-23
- Drafted design vision outlining UI goals
- Added Card component and new Marketplace and Vector DB pages
- Navigation links to Marketplace and Vector DBs


## 2025-09-24
- Added workflows API routes and frontend page to create, run, and delete workflows
- Navigation updated with Workflows link
## 2025-09-25
- Added Organizations page and API integration for creating and listing orgs
- Navigation links to Orgs for quick access
- Updated global styles to use Geist font and adjusted Nav colors
- Hero section now features an indigo gradient background for visual impact

## 2025-09-26
- Home page redesigned with feature cards and a global footer per design vision
- Added FeatureCard and Footer components using Tailwind for consistent styling
- Layout now includes the footer on all pages for better navigation

## 2025-09-27
- Implemented Input and Textarea components inspired by shadcn/ui for consistent form styling.
- Updated login, registration, and new server pages to use these components with card-like form layout.
- Documented design updates and next frontend work.

## 2025-09-28
- Added musikconnect metadata comments to reusable components for automated tooling.
- Created components/README describing Button, Input, Card and other UI pieces.

## 2025-09-29
- Added Ingestion page to manage ingestion jobs using vector DBs
- Navigation links to Ingestion for easy access
- Documented ingestion page usage in folder README

## 2025-09-30
- Added evaluation scoreboard listing recent results across all servers via new /api/evaluations endpoint and UI page

## 2025-10-01
- Added server score summary endpoint and improved Evaluations page to rank servers by average score using Card component.

## 2025-10-02
- Added user profile page showing email, role, and server quota
- `/api/me` now returns server_quota
- Navigation links to Profile page when logged in
## 2025-10-25
- Replaced Docker CLI tagging/pushing with Bollard APIs that stream registry progress into build logs.
- Registry failures now bubble structured errors so build status flips to error with actionable messaging.
- Introduced a logging sink trait and registry push tests to cover success and failure flows.
- Hardened registry push telemetry with scope-aware tracing, digest logging, and retry logic plus auth-expiration handling tests.
- Added tagging-stage metrics (`tag_started`, `tag_succeeded`) and ensured `push_failed` events are emitted for pre-push failures so dashboards can distinguish tagging faults from registry stream errors.

## 2025-10-26
- Audited usage-metrics consumers and updated the server dashboard to render tagging and push telemetry with friendly labels.
- Added `MetricsEventList` to expose new registry metadata alongside charted cadence trends.
- Extended backend registry tests with table-driven coverage for `record_push_failure` and error classification to guard retry/auth flags.
## 2025-10-27
- Verified downstream telemetry consumers: server metrics API/UI accept `tag_*` and enriched `push_*` payloads; updated MetricsEventList to surface registry endpoint, retry, and auth context for failure/retry events.
- Documented payload contract (`attempt`, `retry_limit`, `registry_endpoint`, `error_kind`, `auth_expired`) in README so dashboards ingesting raw JSON stay aligned.
- Added negative-path regression tests covering every `RegistryPushError` variant, zero-retry handling, and malformed remote detail responses to ensure `record_push_failure` metrics remain stable.

## 2025-10-28
- Audited non-UI telemetry consumers (usage_metrics table, REST endpoint, SSE stream) to confirm support for `tag_*` and enriched `push_*` payloads.
- Added regression test ensuring the metrics broadcast delivers full registry telemetry details to downstream listeners.
- Updated README with a telemetry consumer matrix and marked BE-BUILD-004 complete in the tracker.
## 2025-10-29
- Added automated registry credential refresh workflow with shared Docker client guard and retry-loop integration.
- Recorded new telemetry events (`auth_refresh_started`, `auth_refresh_succeeded`, `auth_refresh_failed`) and annotated `push_retry` with `reason="auth_refresh"` for refreshed attempts.
- Extended backend tests to cover refresh success/failure flows and updated README/runbook plus tracker entry BE-BUILD-005.
