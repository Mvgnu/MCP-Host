# Frontend Design Vision

This document outlines the design direction for MCP Host. The goal is a modern, clean interface that showcases the platform's capabilities while remaining accessible.

## Principles
- **Clarity first** – pages should present a single clear action.
- **Consistency** – use shared components and a cohesive color palette.
- **Responsiveness** – design mobile‑first with Tailwind's utilities.
- **Component driven** – build UI from reusable pieces inspired by shadcn/ui.

## Visual Style
- Base colors: slate background with indigo accents.
- Typography: Geist Sans and Mono fonts.
- Buttons and cards use subtle shadows and rounded corners.

## Planned Components
- `Card` – container with padding and border used for lists and feature highlights.
- `Section` – page section with heading and optional description.
- `Button` – primary and secondary variants with disabled state.

## Pages
1. **Landing** – hero banner, feature cards linking to Docs, Blog, and Marketplace.
2. **Marketplace** – list of prebuilt MCP images pulled from `/api/marketplace` using the `Card` component.
3. **Vector DBs** – manage managed vector databases using `/api/vector-dbs` endpoints.
4. **Dashboard** – servers list, capabilities, logs, metrics, and evaluation results.
5. **Lifecycle & Policy Console** – surfaces runtime policy decisions, promotion gates, and evaluation requirements backed by the new `RuntimePolicyEngine`. Operators should be able to inspect recent `runtime_policy_decisions` records, trigger staged rollouts, and visualize tier/health rationale derived from the marketplace ledger.

This vision will guide iterative enhancements to deliver a polished, professional frontend.

## Lifecycle analytics schema

To ensure the console renders a trustworthy automation narrative, lifecycle run snapshots now expose a canonical analytics payload:

```json
{
  "duration_seconds": 180,
  "duration_ms": 182000,
  "execution_window": {"started_at": "2025-12-08T09:52:00Z", "completed_at": "2025-12-08T09:55:00Z"},
  "retry_attempt": 2,
  "retry_limit": 5,
  "retry_count": 2,
  "retry_ledger": [
    {"attempt": 1, "status": "failed", "reason": "timeout", "observed_at": "2025-12-08T09:53:00Z"},
    {"attempt": 2, "status": "succeeded", "observed_at": "2025-12-08T09:55:00Z"}
  ],
  "override_reason": "manual approval",
  "manual_override": {"reason": "manual approval", "actor_email": "operator@example.com"},
  "artifact_fingerprints": [
    {"manifest_digest": "sha256:artifact", "fingerprint": "4c4d5c8a4b341f6a9c5e2d5876a9c1f2"}
  ],
  "promotion_verdict": {
    "verdict_id": 91,
    "allowed": false,
    "stage": "production",
    "track_name": "stable",
    "track_tier": "tier-1"
  }
}
```

Components in `frontend/components/console/` should surface this data through retry badges, override callouts, fingerprint chips, and promotion verdict indicators so operators can quickly interpret automation state across the web console and CLI.
