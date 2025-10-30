<!-- key: federated-data-plane -> governance -->
# Federated Data-Plane Governance

## Overview
Federated attachments allow customer-managed workloads to reference managed vector databases while preserving residency controls and BYOK compliance. This document captures the control-plane contracts introduced in migration `0046_vector_db_governance.sql` and the runtime behaviors enforced by `backend/src/vector_dbs.rs`.

## Data Model
- **`vector_db_residency_policies`** — Declares residency posture for each managed vector database. Uniquely keyed by `(vector_db_id, region)` with classification + enforcement metadata and an `active` toggle. Attachments must reference an active residency policy before persisting.
- **`vector_db_attachments`** — Records federated service attachments alongside a required `provider_key_binding_id`. Attachments only succeed when the referenced binding targets the `vector_db` scope and remains unrevoked. Metadata is stored as JSONB for downstream consoles.
- **`vector_db_incident_logs`** — Captures residency and compliance incidents with optional attachment references, severity tagging, and structured notes to power operational runbooks.

## API Surface
All routes scope access to the vector DB owner via `AuthUser`.
- `POST /api/vector-dbs/:id/residency-policies` (`vector-dbs-residency-policy`) — Upserts residency envelopes. Inactive policies prevent new attachments until toggled.
- `POST /api/vector-dbs/:id/attachments` (`vector-dbs-attachment`) — Validates residency policy activeness and BYOK binding scope before persisting an attachment. `GET /api/vector-dbs/:id/attachments` returns the attachment ledger ordered by `attached_at`, enriched with provider key identifiers and rotation deadlines.
- `PATCH /api/vector-dbs/:id/attachments/:attachment_id` — Marks attachments as detached with operator-supplied reasons once remediation or credential rotation removes the workload.
- `POST /api/vector-dbs/:id/incidents` (`vector-dbs-incident-log`) — Logs compliance incidents after confirming attachment ownership. `GET /api/vector-dbs/:id/incidents` provides an ordered incident history for dashboards and runbooks.
- `PATCH /api/vector-dbs/:id/incidents/:incident_id` — Resolves incidents by stamping `resolved_at` and optional resolution notes, returning HTTP 409 when operators attempt to resolve already-closed entries.

## Operational Playbooks
1. **Residency onboarding** — Operators create a vector DB, upsert residency policies for required regions, and ensure BYOK bindings are registered against provider keys.
2. **Attachment rollout** — Service owners request attachments with approved residency policies and validated BYOK bindings. Attempts with inactive policies or incorrect binding scopes are rejected with actionable HTTP 409 messages. When remediation completes, operators call `PATCH /api/vector-dbs/:id/attachments/:attachment_id` to record the detach reason and timestamp.
3. **Incident triage** — When incidents are logged, responders review `/api/vector-dbs/:id/incidents`, correlate attachment metadata, and use BYOK binding references to rotate or revoke compromised credentials. Structured notes should capture remediation status for audit exports, and closing the loop happens through `PATCH /api/vector-dbs/:id/incidents/:incident_id` with resolution summaries.

## CLI Parity
- `mcpctl vector-dbs attachments detach VECTOR_DB_ID ATTACHMENT_ID [--reason TEXT]` mirrors the detachment API so headless operators can record separation events with optional reasoning.
- `mcpctl vector-dbs incidents resolve VECTOR_DB_ID INCIDENT_ID [--summary TEXT] [--notes JSON]` resolves incidents without the console, parsing structured JSON notes to keep audit metadata aligned with console submissions.

## Testing Guidance
Regression coverage in `backend/tests/vector_dbs.rs` seeds residency policies, exercises invalid binding scenarios, and asserts that incidents referencing foreign attachments return HTTP 404. Detachment and incident resolution tests now confirm HTTP 409 conflicts and that rotation deadlines flow through attachment listings. Extend the suite with residency rotation cases as remediation workflows mature.
