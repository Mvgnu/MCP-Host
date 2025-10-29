# Problem_Tracker_NextIteration

- ID: FE-LIFE-STREAM-001
  Status: DONE
  Task: Ensure lifecycle console SSE connections include credentials.
  Hypothesis: Lifecycle SSE streams authenticated by cookies disconnect without credentials; enabling `withCredentials` should keep sessions alive alongside existing REST fetch configuration.
  Log:
    - 2025-10-29 17:30:00 UTC: Investigated operator report of lifecycle stream dropping when cookies guard the SSE endpoint; confirmed EventSource omitted credentials.
    - 2025-10-29 17:40:00 UTC: Added credentialed EventSource init and kept snapshot polling fallback to guarantee continuity.
