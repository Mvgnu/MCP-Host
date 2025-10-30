<!-- key: provider-marketplace -> journey-guide -->
# Provider Marketplace Journey Guide

## Purpose
This guide maps the provider-facing marketplace journey from initial submission through promotion into production. It aligns the console, CLI, and backend surfaces that were introduced in `backend/src/marketplace.rs`, `frontend/app/console/marketplace/providerdashboard/page.tsx`, and `cli/mcpctl/commands/__init__.py` so that commercialization, compliance, and operations teams share a single reference.

## Lifecycle Overview
1. **Submission** – Providers call `POST /api/marketplace/providers/:provider_id/submissions` (via console wizard or `mcpctl marketplace submissions create`). The backend validates BYOK posture (`ProviderKeyService::summarize_for_policy`) and persists submission posture, metadata, and release notes. Successful submissions emit `submission_created` events over `/api/marketplace/providers/:provider_id/events/stream` for console and CLI watchers.
2. **Evaluation** – Reviewers launch evaluations with `POST /api/marketplace/providers/:provider_id/submissions/:submission_id/evaluations` (`mcpctl marketplace evaluations start`). Evaluation transitions (`.../evaluations/:evaluation_id/transition`) capture results, posture notes, and completion timestamps. Each transition logs `evaluation_transitioned` audit events and SSE payloads so downstream automation can reconcile state.
3. **Promotion gating** – Promotion gates are created through `POST /api/marketplace/providers/:provider_id/evaluations/:evaluation_id/promotions` (new `mcpctl marketplace promotions create`). Transitions land at `POST /api/marketplace/providers/:provider_id/promotions/:promotion_id/transition` (`mcpctl marketplace promotions transition`). Gate changes stream to the SSE feed with `promotion_created` and `promotion_transitioned` events, unlocking runtime promotion automation.
4. **Runtime placement** – Once promotion gates reach an approved/ready status, promotion records surface through `/api/marketplace/providers/:provider_id/submissions` and the lifecycle console overlays. Runtime policy consumers (`backend/src/policy.rs`) load these promotion snapshots to validate placement eligibility.

## Compliance Checkpoints
- **BYOK posture verification** – Submission handlers reject providers without active keys, and posture notes are captured on every submission/evaluation/promotion record (`posture_vetoed`, `posture_notes`). Operators must resolve BYOK vetoes before advancing evaluations.
- **Evaluation evidence capture** – Evaluation `result` payloads store structured JSON evidence (e.g., security scan results). Reviewers should attach evaluator references and scores; failing evaluations should annotate posture notes for audit trails.
- **Promotion gate notes** – Every promotion gate accepts repeatable notes. Use them to document risk waivers, remediation follow-up, and dependency acknowledgements. The CLI normalizes repeated `--note` flags and the console surfaces them within `MarketplaceSubmissionCard` timelines.
- **Audit fan-out** – `provider_marketplace_events` captures actor references and timestamps for every transition. Compliance analysts can replay these events via `mcpctl marketplace watch PROVIDER_ID` with `--json` for machine parsing.

## Fail-Forward & Recovery Paths
- **Submission veto** – If a submission is vetoed (missing BYOK, invalid manifest), fix the underlying issue and resubmit. Historical submissions remain queryable for audit but the console highlights veto reasons sourced from `posture_notes`.
- **Evaluation failure** – Use `mcpctl marketplace evaluations transition ... --status retrying` (or similar) to reflect re-runs. Attach remediation notes to track why an evaluation stalled. Failed evaluations can spawn new promotion gates once remediated evidence is collected.
- **Promotion rejection** – When `promotion_transitioned` events return a veto or rejection, append follow-up notes explaining the outcome. Create a new promotion gate after remediation—previous gates remain in the timeline for traceability.
- **SSE disruption** – The console and CLI automatically reconnect to `/events/stream` with credentials. If event drift occurs, re-fetch `/submissions` for authoritative state and reconcile differences before approving promotions.

## Operator Touchpoints
- **Console dashboard** – `/console/marketplace/providerdashboard` renders submission cards with real-time SSE updates, artifact upload helpers, and BYOK posture badges.
- **CLI workflows** – `mcpctl marketplace submissions|evaluations|promotions` mirror console operations for headless providers. `mcpctl marketplace watch` provides real-time event feeds with summarized context for terminal workflows.
- **Documentation hygiene** – Update this guide whenever backend contracts evolve (e.g., artifact upload payloads, new promotion statuses) so commercialization pilots have actionable, current references.
