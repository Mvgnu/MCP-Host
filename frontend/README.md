This is a [Next.js](https://nextjs.org) project bootstrapped with [`create-next-app`](https://nextjs.org/docs/app/api-reference/cli/create-next-app).

## Getting Started

First, run the development server:

```bash
npm run dev
# or
yarn dev
# or
pnpm dev
# or
bun dev
```

Open [http://localhost:3000](http://localhost:3000) with your browser to see the result.

### Lifecycle Console

- Navigate to `/console/lifecycle` to view the remediation lifecycle console. The page orchestrates REST pagination with an SSE
  stream exposed at `/api/console/lifecycle/stream`, reconnecting automatically with cursor resumption metadata.
- Snapshot events drive workspace cards that surface promotion gate verdicts, recent remediation runs, trust registry states,
  capability intelligence scores, and marketplace readiness in a single view.
- Promotion verdict metadata now renders through the `PromotionVerdictTimeline` component, highlighting the latest
  `promotion_postures` array (track, stage, allowed flag, veto reasons, remediation hooks, and signal JSON) with delta badges
  whenever SSE events update the posture.
- Promotion automation refreshes now appear alongside posture narratives. The console renders the `promotion_runs` table with
  gate lane/stage context, automation payload attempts, and a change summary sourced from SSE `promotion_run_deltas` so
  operators can correlate veto narratives with the remediation orchestration they triggered.
- Filter controls (workspace search, promotion lane, severity) feed the backend query parameters and persist to local storage,
  enabling scoped investigations and shared context between browser sessions.
- Snapshot envelopes include delta metadata. The UI renders recent trust/intelligence/marketplace changes on each run and opens a
  drill-down modal with threaded timelines, metadata, and replay context.
- When streaming is unavailable or the browser goes offline the UI falls back to a 15s REST polling loop. Cursor state, filters,
  recent snapshots, and run deltas hydrate from local storage so operators can resume where they left off once connectivity
  returns.
- Playwright regression coverage for filters, offline resume, and SSE replay semantics lives in `e2e/console.spec.ts`; run
  `npx playwright test e2e/console.spec.ts` against a running development server to capture updated expectations.

This project uses [`next/font`](https://nextjs.org/docs/app/building-your-application/optimizing/fonts) to automatically optimize and load [Geist](https://vercel.com/font), a new font family for Vercel.

## Learn More

To learn more about Next.js, take a look at the following resources:

- [Next.js Documentation](https://nextjs.org/docs) - learn about Next.js features and API.
- [Learn Next.js](https://nextjs.org/learn) - an interactive Next.js tutorial.

You can check out [the Next.js GitHub repository](https://github.com/vercel/next.js) - your feedback and contributions are welcome!

## Deploy on Vercel

The easiest way to deploy your Next.js app is to use the [Vercel Platform](https://vercel.com/new?utm_medium=default-template&filter=next.js&utm_source=create-next-app&utm_campaign=create-next-app-readme) from the creators of Next.js.

Check out our [Next.js deployment documentation](https://nextjs.org/docs/app/building-your-application/deploying) for more details.
