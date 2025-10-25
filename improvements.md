# Improvement Tracker

This file lists all planned improvements and fixes based on initial code review.
Completed items are checked.

- [x] Fix webhook secret to use header instead of query param
- [x] Add database indexes for foreign key columns
- [x] Improve backend error logging with `tracing`
- [x] Require `JWT_SECRET` env var at startup
- [x] Add basic backend and frontend tests

## Phase 2: Refactor & Decouple
- [x] Replace shell commands with `git2` and `bollard::Docker::build_image`
- [x] Replace Nginx/Certbot shelling with a safer approach
- [x] Introduce state management on the frontend (e.g. Zustand) and SSE for status
- [x] Break down monolithic components (e.g. `ServerCard` etc.)

## Phase 3: Mature & Scale
- [x] Decouple control plane and worker using a job queue
- [x] Adopt a centralized secrets manager
- [x] Add comprehensive test coverage and CI pipeline
- [x] Package CLI tools for distribution
 - [x] Implement RBAC roles and server quotas
