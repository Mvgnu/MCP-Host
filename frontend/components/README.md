# Frontend Components

This directory contains reusable UI components used across the MCP Host frontend.

- **Button** – styled button or link with primary and secondary variants
- **Input** – text input with optional label
- **Textarea** – textarea input with optional label
- **Card** – generic container for lists and highlights
- **ServerCard** – dashboard item showing an MCP server with action buttons
- **FeatureCard** – card used on the home page to showcase features
- **MetricsChart** – chart.js wrapper for server metrics, aware of tagging (`tag_*`) and push (`push_*`) events
- **MetricsEventList** – textual timeline surfacing registry telemetry metadata, including retry/auth context for registry pushes
- **Spinner** – small loading indicator
- **Alert** – error message display
- **onboarding/**
  - **SelfServiceOnboarding** – renders the self-service funnel for administrators bootstrapping organizations.
  - **AcceptInvitation** – guides invited teammates through sign-in or registration before accepting their invitation token.
- **console/**
  - **LifecycleTimeline** – renders remediation run timeline rows with trust/intelligence overlays and wires run-level approvals
  - **LifecycleRunProgress** – highlights run status, timing, approval state metadata, and now exposes approve/reject controls with optimistic state badges
  - **LifecycleTrustOverlay** – surfaces trust registry, intelligence scores, and marketplace readiness for each run
  - **LifecycleVerdictCard** – summarizes active revision gate snapshots and promotion verdicts
  - **BillingPlanComparison** – renders billing plans with entitlement summaries and selection affordances for the console subscription wizard
  - **MarketplaceSubmissionCard** – surfaces submission posture badges, evaluation timelines, and promotion notes for provider marketplace reviewers
  - **VectorDbResidencyCard** – manages residency policy lists and inline upsert forms for federated vector databases
  - **VectorDbAttachmentList** – displays attachment metadata with credential rotation badges and detachment controls
  - **VectorDbIncidentTimeline** – renders incident history with remediation forms to resolve breaches
  - **Tests** – `console/__tests__/LifecycleRunProgress.test.tsx` validates analytics chip formatting and duration fallbacks

These components follow the design principles in `../../design-vision.md` and
include musikconnect tags for tooling.
