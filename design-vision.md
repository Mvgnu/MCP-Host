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

This vision will guide iterative enhancements to deliver a polished, professional frontend.
