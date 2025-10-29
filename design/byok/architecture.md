# Provider BYOK Secrets Fabric Architecture

## Objective
Deliver tenant/provider-scoped key management that satisfies compliance requirements for managed SaaS customers while integrating with existing runtime policy, governance, and lifecycle tooling. The fabric must let providers register keys, enforce rotations, gate runtime launches, and expose posture across REST, CLI, and console surfaces without leaking raw secret material.

## Guiding Principles
- **Provider isolation** – Keys, audit trails, and runtime gates are partitioned by provider (or tenant) and never shared across organizations.
- **Contract parity** – REST, SSE, CLI, and UI surfaces emit the same metadata envelopes, mirroring the structure used by governance and remediation modules.
- **Optimistic concurrency** – Every mutation carries version tokens that protect concurrent edits in the same fashion as existing governance and promotion routes.
- **Machine-readable documentation** – Backend modules include `key:` metadata comments so downstream automation can locate contracts.
- **Event durability** – Rotations, expirations, and policy vetoes emit durable audit events for compliance review and operator alerting.

## Domain Model & Persistence
Key persistence lives in new migrations alongside rotation and binding tables. Planned tables:
- `provider_keys` – stores key metadata (provider_id, key_id, alias, key_material_reference, attestation_digest, attestation_signature presence, attestation_verified_at, state enum, created_at, activated_at, retired_at, compromised_at, rotation_due_at, version, audit_actor_ref).
- `provider_key_rotations` – history of rotation attempts (rotation_id, provider_key_id, submitted_at, approved_at, rotation_state enum, evidence_uri, attestation_digest, attestation_signature presence, request_actor_ref, approval_actor_ref, failure_reason, metadata JSONB).
- `provider_key_bindings` – mapping from key versions to artifacts/workspaces/policy decisions (provider_key_id, binding_type enum, binding_target_id, binding_scope JSONB, created_at, revoked_at, revoked_reason, version).
- `provider_key_audit_events` – durable event log for compliance (event_id, provider_id, provider_key_id, event_type, event_payload JSONB, occurred_at).

Migration strategy:
1. Introduce schema via additive migration `0041_provider_keys.sql` (following style of `0022-0025`). Include downgrade path removing tables.
2. Seed existing providers with placeholder `state = 'pending-registration'` rows to enforce policy gating immediately after deployment.
3. Add NOTIFY triggers on `provider_key_audit_events` for SSE rebroadcasts, mirroring remediation/governance listeners.

## Backend Module Structure
Create a dedicated module tree under `backend/src/keys/`:
- `mod.rs` – exposes `ProviderKeyService` with CRUD, rotation state transitions, and audit emission helpers. Annotate with `// key: provider-keys-service` contract comment.
- `models.rs` – typed structs/enums for database rows, `ProviderKeyState`, and binding types.
- `events.rs` – durable audit event definitions and serialization.
- `policy.rs` – helpers consumed by runtime policy engine to hydrate key posture and emit veto notes (`key: provider-keys-policy`).

Service responsibilities:
- Register keys via public key material references or attestation bundles (raw material never stored).
- Enforce state machine transitions: `pending_registration → active → rotating → retired/compromised` with optimistic version tokens.
- Persist rotation requests and approvals, capturing operator identity from auth context.
- Publish audit events for registration, activation, rotation scheduled/completed, compromise declared, runtime veto triggered, attestation uploaded, and binding changes.

## Runtime & Policy Enforcement
- Extend `RuntimePolicyEngine` to load provider key posture when evaluating launches for tiers that declare `byok_required = true`.
- Implementation status: migration `0042_provider_tier_requirements.sql` introduces the `provider_tiers` lookup and expands
  `runtime_policy_decisions` with a `key_posture` JSONB column. The policy engine now hydrates active keys for BYOK tiers,
  records `provider-key:*` notes, and persists a `ProviderKeyDecisionPosture` envelope (provider/key identifiers, state,
  rotation deadline, veto notes, and attestation evidence flag) so SSE clients and CLI streams can surface posture alongside
  governance/evaluation notes.
- Augment `runtime_policy_decisions` payload with `key_posture` metadata (`state`, `rotation_due_at`, `attestation_digest`, `veto_reason`). Use the same SSE feed as governance to surface vetoed launches.
- Update `backend/src/runtime.rs` orchestrator to block launch when posture is unhealthy, emitting structured failure reasons and scheduling remediation events if rotation overdue.
- Status: Runtime orchestrator blocks deployments when the evaluated posture is vetoed, updates server status to a posture-specific pending state, and writes a `runtime_veto` audit record via the key service so SSE consumers can react.
- Integrate with lifecycle aggregation so `/api/console/lifecycle` includes key posture badges and rotation debt counters.

## REST & SSE Surface
Add new routes with optimistic locking semantics:
- `POST /api/providers/:id/keys` – register/replace active key using attestation bundle, returning posture snapshot and version.
- `POST /api/providers/:id/keys/:key_id/rotations` – submit rotation request with evidence, returns rotation ticket and flips the key into `rotating` posture pending approval.
- `POST /api/providers/:id/keys/:key_id/approve` – approve rotation after attestation.
- `GET /api/providers/:id/keys` – list keys, posture summaries, binding counts.
- `GET /api/providers/:id/keys/:key_id` – detailed state, rotation history, audit trail slice.
- `GET /api/providers/:id/keys/stream` – SSE snapshots referencing audit events and posture changes, reusing `withCredentials` SSE style.

Handler layout:
- Add `backend/src/keys_api.rs` to host handlers (`// key: provider-keys-api`).
- Wire routes inside `backend/src/routes.rs` with tower layers matching governance/auth patterns.
- Extend SSE broadcaster to subscribe to new NOTIFY channel and encode payload as `ProviderKeyStreamMessage`.

## CLI Contract
Introduce `mcpctl keys` command group with parity features:
- `keys register --provider <id> --attestation <file> --alias <alias>`
- `keys list --provider <id> [--json]`
- `keys rotate --provider <id> --key <key_id> --attestation <file> --actor-ref <ref>`
- `keys approve-rotation --provider <id> --rotation <rotation_id>`
- `keys bindings --provider <id> --key <key_id>`
- `keys watch --provider <id>` (SSE stream)

Implementation steps:
- Add client wrappers under `cli/mcpctl/keys.py` (or analogous module) that reuse existing HTTP client scaffolding.
- Record snapshot tests in `cli/tests/test_keys_*.py` verifying JSON payloads and optimistic-lock headers.
- Update CLI README with usage and parity guarantees.
- Status: `register`, `list`, `rotate`, `bind`, and `bindings` now return live data backed by the REST service; `approve-rotation` and `watch` remain staged as placeholders pending backend orchestration.

## Console & Provider Portal
- Inject BYOK posture into lifecycle console cards inside `frontend/components/console/` with badges for `rotation overdue`, `attestation missing`, and `compromised` states.
- Extend provider marketplace dashboard under `frontend/pages/marketplace/provider/` with BYOK management UI: registration flow (upload attestation bundle), rotation wizard, bindings overview.
- Use encrypted payload handoff: attestation bundles uploaded to backend via signed URLs or streaming POST body; never persist plaintext in client state.
- Apply optimistic UI store updates mirroring CLI contract; reconcile SSE payloads to resolve eventual consistency.

## Security & Compliance
- Require signed attestations referencing hardware security modules or provider-managed KMS. Persist verification timestamps alongside digests so runtime policy can gate on stale evidence.
- Capture operator identity (user_id/email) for every mutation; persist to audit events and enforce non-empty actor references for rotations.
- Validate attestation freshness windows and enforce `rotation_due_at` semantics.
- Ensure runtime gating occurs before workloads access secrets or data-plane attachments.
- Integrate with secrets service to map provider key IDs to vault handles when BYOK extends to runtime encryption.

## Testing Strategy
- **Backend integration tests**: extend SQLx fixtures to cover registration, rotation happy path, compromised key veto, policy gate refusal, SSE emission ordering.
  - 2025-12-12: Added runtime policy regression coverage asserting BYOK vetoes when keys are absent and healthy posture when attested keys exist.
- **CLI tests**: snapshot JSON for list/register/rotate/approve/bindings flows; simulate optimistic locking errors.
- **Frontend tests/stories**: add Jest/Storybook coverage for console badges and provider portal flows with mocked SSE payloads.
- **Migration tests**: update migration harness to ensure downgrade path succeeds and triggers preserve NOTIFY semantics.

## Rollout Plan
1. Land schema migration and backend scaffolding behind feature flag (`BYOK_ENABLED`).
2. Deploy backend with routes returning `501` until CLI/UI ready; update documentation.
3. Release CLI contract with experimental flag; collect provider feedback.
4. Wire runtime gating and lifecycle analytics; enable enforcement for pilot providers.
5. Activate provider portal flows and finalize compliance documentation.
