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

These components follow the design principles in `../../design-vision.md` and
include musikconnect tags for tooling.
