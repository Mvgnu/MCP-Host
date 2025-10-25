# Refinement Plan

This document tracks the next phase of improvements following the comprehensive audit.

## Phase 1 – Hardening & Immediate Refinements
- [x] Modular routing: split the monolithic router in `main.rs` into a dedicated module.
- [x] Structured error handling via a central `AppError` type with `IntoResponse` support.
- [x] CI enhancements running clippy, fmt, lint and cargo audit.
- [x] Frontend API hooks using SWR/TanStack Query.

## Phase 2 – Production Readiness
- [x] Proxy isolation: move Nginx/TLS management to a separate controller.
### 2025-09-16
- Implemented workflows to chain MCP servers with sequential invocation.

### 2025-09-17
- Implemented invocation tracing with new database table.
- Added endpoint and UI page to review past invocations.

### 2025-09-18
- Added automated evaluation endpoints to create tests and run them against servers.
- Results are stored with similarity scores for quality tracking.

### 2025-09-19
- Added basic organization support with new tables and API routes.
- Servers may specify an `organization_id` during creation.

### 2025-09-20
- Improved invocation logging by reporting database errors
- Marked Phase 1 items as complete
\n### 2025-09-21
- Replaced unsafe header unwraps with expect in auth module\n
\n### 2025-09-22\n- Added frontend pages for evaluation tests and results, integrating new API endpoints.

### 2025-09-23
- Added design vision document and initial marketplace/vector DB pages using new Card component.

### 2025-09-24
- Implemented workflow management routes and UI page with Tailwind components
### 2025-09-25
- Added organizations management page utilizing existing API endpoints
- Applied design vision updates: fonts now use Geist across the site and navigation adopts slate tones
- Enhanced Hero component with gradient styling for a more modern landing page

### 2025-09-26
- Added FeatureCard and Footer components to implement design vision
- Home page now highlights Marketplace, Vector DBs, and Workflows
- Footer appears site-wide for consistent navigation

### 2025-09-27
- Introduced shadcn-inspired Input and Textarea components for improved form UX.
- Login, register, and new server pages adopt a card layout with these components.


### 2025-09-29
- Added Ingestion page exposing ingestion job endpoints and vector DB selection
- Navigation updated with Ingestion link for full feature coverage

### 2025-09-30
- Implemented evaluation scoreboard with new backend route and page

### 2025-10-01
- Added score summary endpoint returning per-server averages and updated the Evaluations page to display rankings

### 2025-10-02
- Added profile page using new server quota data from `/api/me`
