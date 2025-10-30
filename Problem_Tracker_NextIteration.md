# Problem_Tracker_NextIteration

- ID: FE-LIFE-STREAM-001
  Status: DONE
  Task: Ensure lifecycle console SSE connections include credentials.
  Hypothesis: Lifecycle SSE streams authenticated by cookies disconnect without credentials; enabling `withCredentials` should keep sessions alive alongside existing REST fetch configuration.
  Log:
    - 2025-10-29 17:30:00 UTC: Investigated operator report of lifecycle stream dropping when cookies guard the SSE endpoint; confirmed EventSource omitted credentials.
    - 2025-10-29 17:40:00 UTC: Added credentialed EventSource init and kept snapshot polling fallback to guarantee continuity.
- ID: BYOK-001
  Status: DONE
  Task: Stage provider BYOK architecture and scaffolding.
  Hypothesis: Documenting the end-to-end design and adding placeholder migrations/modules will align backend, CLI, and console workstreams before full enforcement lands.
  Log:
    - 2025-12-12 09:10:00 UTC: Authored architecture note, added persistence scaffolding migration, and stubbed backend service/API routes for provider key management.
    - 2025-12-12 11:20:00 UTC: Implemented attestation-aware registration with rotation deadlines across backend, CLI, and docs to prepare for runtime enforcement wiring.
    - 2025-12-12 13:05:00 UTC: Delivered rotation request flow with audit events, CLI hashing, and REST contract updates to flip keys into rotating posture while approvals are staged.
    - 2025-12-12 14:45:00 UTC: Enforced BYOK gating for flagged tiers, persisted `key_posture` metadata on policy decisions, and updated CLI/console contracts to stream provider key posture alongside governance notes.
    - 2025-12-12 16:10:00 UTC: Runtime orchestrator blocks launches on vetoed posture, records `runtime_veto` audit events, and surfaces posture-specific pending statuses for operators.
    - 2025-12-12 17:25:00 UTC: Required signed attestation bundles with verification timestamps, enforced rotation actor capture, and propagated signature posture across backend, CLI, and docs.
    - 2025-12-12 18:40:00 UTC: Lifecycle console SSE now streams provider key posture and deltas; UI badges, overlays, and tests surface BYOK state, vetoes, and attestation notes alongside trust analytics.
    - 2025-12-12 19:55:00 UTC: Delivered provider key binding persistence, REST/CLI endpoints, audit fan-out, and regression tests so workloads can declare BYOK coverage without leaving the CLI.
    - 2025-12-12 21:10:00 UTC: Landed runtime policy regression tests that assert BYOK vetoes and healthy paths, confirming SSE posture data aligns with enforcement.
    - 2025-12-15 08:30:00 UTC: Implemented rotation SLA enforcement helpers, emergency revocation APIs, scoped audit filters, CLI parity (revoke/audit), and regression coverage to close governance/compliance gaps.
    - 2025-12-15 09:20:00 UTC: Enabled SQLx UUID feature support, deduplicated rotation/revocation handlers, and updated policy/runtime tests so BYOK governance suite builds and passes end-to-end.

- ID: LC-CTRL-002
  Status: DONE
  Task: Activate lifecycle console control loops with optimistic actions.
  Hypothesis: Wiring promotion and remediation commands into the console with optimistic state should keep operators in-flow while SSE reconciles final state.
  Log:
    - 2025-10-29 22:17:38 UTC: Added lifecycle action client hook, promotion buttons, and run approval controls with optimistic rollbacks and pending badges.

- ID: SAAS-BILLING-001
  Status: IN_PROGRESS
  Task: Stand up SaaS subscriptions, entitlements, and quota enforcement.
  Hypothesis: Normalizing plan/subscription tables, exposing BillingService quotas, and wiring CLI onboarding will unblock commercialization pilots and policy gating.
  Log:
    - 2025-12-16 09:05:00 UTC: Added billing migrations, BillingService quota enforcement, runtime policy gating notes, billing REST endpoints, CLI commands, and tests to enable organization subscription bootstrap and entitlement checks.
    - 2025-12-17 10:15:00 UTC: Activated billing reconciliation worker to settle provider usage callbacks, expanded provider adapter normalization, surfaced downgrade/suspension helpers in `BillingService`, documented lifecycle flows, and wired the worker into the main server/router so usage ledger entries reconcile asynchronously.
    - 2025-12-18 08:45:00 UTC: Delivered plan catalog API + console onboarding wizard with plan comparison, trial toggles, and real-time `BillingQuotaOutcome` previews so operators can assign plans and stage downgrades directly from the console while maintaining CLI parity.
    - 2025-12-18 14:55:00 UTC: Added SQLx-backed billing regression tests for multi-entitlement quotas, documented provisioning/renewal/cancellation diagrams, and confirmed runtime veto messaging stays actionable when subscriptions lapse or quotas are exceeded.
    - 2025-12-21 10:20:00 UTC: Landed renewal automation scheduler with configurable grace windows, fallback downgrades, suspension paths, and SQLx coverage so overdue accounts transition without relying on provider callbacks.
    - 2025-12-22 08:35:00 UTC: Added self-service onboarding APIs, invitation acceptance flows, and public onboarding UI so administrators can register, create organizations, pick plans, and invite teammates without operator intervention.
    - 2025-12-22 16:10:00 UTC: Delivered invite acceptance UI at `/onboarding/invite/[token]` with registration/login helpers, API error handling, and regression tests to cover happy-path and failure cases.
  Risks:
    - Renewal downgrade flow still depends on provider callbacks to mark accounts `past_due`; we need an automated chron job before GA.

- ID: MARKETPLACE-001
  Status: IN_PROGRESS
  Task: Launch provider marketplace submission/evaluation backend.
  Hypothesis: Gating submissions on BYOK posture, persisting evaluation/promotion transitions, and emitting audit events will unblock console and CLI marketplace experiences without sacrificing compliance controls.
  Log:
    - 2025-12-18 18:05:00 UTC: Added `0045_provider_marketplace.sql`, wired new Axum handlers for submissions/evaluations/promotions with BYOK posture checks, registered router routes, and documented the surface alongside SQLx-backed regression coverage that exercises happy-path flows plus veto enforcement.
    - 2025-12-19 09:15:00 UTC: Broadcast provider marketplace events over a credentialed SSE feed, added SQLx regression coverage to assert stream fan-out, and delivered the console provider dashboard with artifact upload, posture badges, and evaluation timelines auto-refreshing from the stream.
    - 2025-12-19 14:20:00 UTC: Landed `mcpctl marketplace` commands for submissions, evaluations, and SSE watch, added CLI regression tests, and updated operator docs so headless providers can push manifests and monitor evaluation outcomes without the console.
    - 2025-12-19 17:40:00 UTC: Extended `mcpctl marketplace` with promotion gate create/transition flows, normalized note handling, refreshed CLI tests/docs, and published a provider journey guide documenting submission-to-production checkpoints.
  Next:
    - Wire provider artifact upload helpers once backend artifact storage lands.

- ID: FED-DATA-PLANE-001
  Status: IN_PROGRESS
  Task: Federate vector DB governance with residency enforcement.
  Hypothesis: Adding residency-aware attachments, BYOK binding enforcement, and incident logging will unlock self-service governance while preserving compliance posture for federated data planes.
  Log:
    - 2025-12-20 09:40:00 UTC: Landed `0046_vector_db_governance.sql`, exposed residency/attachment/incident routes in `vector_dbs`, enforced active residency + `vector_db` binding scopes, documented the architecture, and added SQLx-backed tests covering conflict + incident paths.
    - 2025-12-20 16:20:00 UTC: Added attachment detachment + incident resolution APIs with rotation metadata joins, expanded SQLx tests for detach/resolution flows, delivered the console governance page with residency/attachment/incident surfaces, and updated design/docs to capture the new operator workflows.
    - 2025-12-20 21:55:00 UTC: Delivered `mcpctl vector-dbs` detach/resolve commands with CLI tests, swapped deprecated base64/Bollard usage for engine/builders to quiet cargo warnings, refreshed CLI docs, and linked proxy controller to the shared proxy module to remove dead code noise.
  Risks:
    - Track CLI automation for residency policy updates (create/list) so operators can fully manage governance without the console path.

- ID: CUST-ONBOARDING-001
  Status: IN_PROGRESS
  Task: Launch customer-facing self-service onboarding funnel.
  Hypothesis: Allowing administrators to self-serve account creation, organization provisioning, plan selection, and teammate invitations will reduce operator toil and accelerate SaaS growth experimentation.
  Log:
    - 2025-12-22 08:35:00 UTC: Delivered `/onboarding` multi-step flow, onboarding component tests, and invitation APIs so self-service sign-ups can complete without manual operator actions.
    - 2025-12-22 16:10:00 UTC: Added `/onboarding/invite/[token]` acceptance route, companion UI, and regression tests so invited teammates can self-serve into organizations once they receive a token.
