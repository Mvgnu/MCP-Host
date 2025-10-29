# Problem_Tracker_NextIteration

- ID: FE-LIFE-STREAM-001
  Status: DONE
  Task: Ensure lifecycle console SSE connections include credentials.
  Hypothesis: Lifecycle SSE streams authenticated by cookies disconnect without credentials; enabling `withCredentials` should keep sessions alive alongside existing REST fetch configuration.
  Log:
    - 2025-10-29 17:30:00 UTC: Investigated operator report of lifecycle stream dropping when cookies guard the SSE endpoint; confirmed EventSource omitted credentials.
    - 2025-10-29 17:40:00 UTC: Added credentialed EventSource init and kept snapshot polling fallback to guarantee continuity.
- ID: BYOK-001
  Status: IN_PROGRESS
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
