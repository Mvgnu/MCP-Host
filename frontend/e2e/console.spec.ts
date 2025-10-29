import { test, expect } from '@playwright/test';

// key: lifecycle-console-ui -> playwright-smoke
const PAGE_PATH = '/console/lifecycle';

const MOCK_PAGE = {
  workspaces: [
    {
      workspace: {
        id: 101,
        workspace_key: 'ops-prod',
        display_name: 'Ops Production',
        description: 'Production remediation workspace',
        owner_id: 7,
        lifecycle_state: 'active',
        active_revision_id: 501,
        metadata: {},
        lineage_tags: ['prod'],
        created_at: new Date().toISOString(),
        updated_at: new Date().toISOString(),
        version: 3,
      },
      active_revision: {
        revision: {
          id: 501,
          workspace_id: 101,
          revision_number: 3,
          previous_revision_id: 500,
          created_by: 3,
          plan: {},
          schema_status: 'passed',
          schema_errors: [],
          policy_status: 'approved',
          policy_veto_reasons: [],
          simulation_status: 'complete',
          promotion_status: 'ready',
          metadata: {},
          lineage_labels: [],
          schema_validated_at: new Date().toISOString(),
          policy_evaluated_at: new Date().toISOString(),
          simulated_at: new Date().toISOString(),
          promoted_at: null,
          created_at: new Date().toISOString(),
          updated_at: new Date().toISOString(),
          version: 9,
        },
        gate_snapshots: [],
      },
      recent_runs: [
        {
          run: {
            id: 9001,
            runtime_vm_instance_id: 123,
            playbook: 'restart-service',
            playbook_id: 44,
            status: 'succeeded',
            automation_payload: {},
            approval_required: false,
            started_at: new Date(Date.now() - 60000).toISOString(),
            completed_at: new Date().toISOString(),
            last_error: null,
            assigned_owner_id: null,
            sla_deadline: null,
            approval_state: null,
            approval_decided_at: null,
            approval_notes: null,
            metadata: { severity: 'high' },
            workspace_id: 101,
            workspace_revision_id: 501,
            promotion_gate_context: {},
            version: 2,
            updated_at: new Date().toISOString(),
            cancelled_at: null,
            cancellation_reason: null,
            failure_reason: null,
          },
          trust: {
            runtime_vm_instance_id: 123,
            attestation_status: 'trusted',
            lifecycle_state: 'ready',
            remediation_state: 'closed-loop',
            remediation_attempts: 1,
            freshness_deadline: null,
            provenance_ref: 'catalog://trust/ops',
            provenance: {},
            version: 4,
            updated_at: new Date().toISOString(),
          },
          intelligence: [
            {
              capability: 'containment',
              backend: 'sentinel',
              tier: 'tier-1',
              score: 0.92,
              status: 'operational',
              confidence: 0.85,
              last_observed_at: new Date().toISOString(),
            },
          ],
          marketplace: {
            status: 'eligible',
            last_completed_at: new Date().toISOString(),
          },
        },
      ],
      promotion_postures: [
        {
          promotion_id: 701,
          manifest_digest: 'sha256:fixture',
          stage: 'production',
          status: 'scheduled',
          track_id: 51,
          track_name: 'Lifecycle',
          track_tier: 'gold',
          allowed: false,
          veto_reasons: ['trust.lifecycle_state=quarantined'],
          notes: ['posture:trust.lifecycle_state:quarantined'],
          updated_at: new Date().toISOString(),
          remediation_hooks: ['hook:remediation.refresh'],
          signals: { trust: { lifecycle_state: 'quarantined' } },
        },
      ],
    },
  ],
  next_cursor: null,
};

const MOCK_DELTA = {
  workspaces: [
    {
      workspace_id: 101,
      run_deltas: [
        {
          run_id: 9001,
          status: 'succeeded',
          trust_changes: [
            {
              field: 'trust.lifecycle_state',
              previous: 'ready',
              current: 'steady',
            },
          ],
          intelligence_changes: [
            {
              field: 'intelligence.containment.score',
              previous: '0.9200',
              current: '0.9500',
            },
          ],
          marketplace_changes: [
            {
              field: 'marketplace.status',
              previous: 'eligible',
              current: 'approved',
            },
          ],
        },
      ],
      removed_run_ids: [],
      promotion_posture_deltas: [
        {
          promotion_id: 701,
          manifest_digest: 'sha256:fixture',
          stage: 'production',
          status: 'scheduled',
          track_id: 51,
          track_name: 'Lifecycle',
          track_tier: 'gold',
          allowed: true,
          veto_reasons: [],
          notes: ['posture:trust.lifecycle_state:restored'],
          updated_at: new Date().toISOString(),
          remediation_hooks: [],
          signals: null,
        },
      ],
      removed_promotion_ids: [],
    },
  ],
};

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    class MockEventSource {
      static instances: MockEventSource[] = [];
      url: string;
      readyState = 0;
      onerror: (() => void) | null = null;
      private listeners: Record<string, ((event: MessageEvent) => void)[]> = {};

      constructor(url: string) {
        this.url = url;
        MockEventSource.instances.push(this);
        (window as any).__mockEventSources = MockEventSource.instances;
      }

      addEventListener(type: string, listener: (event: MessageEvent) => void) {
        if (!this.listeners[type]) {
          this.listeners[type] = [];
        }
        this.listeners[type].push(listener);
      }

      close() {
        this.readyState = 2;
      }

      emit(type: string, payload: unknown) {
        const listeners = this.listeners[type] ?? [];
        const event = { data: JSON.stringify(payload) } as MessageEvent;
        listeners.forEach((listener) => listener(event));
      }
    }

    (window as any).MockEventSource = MockEventSource;
    (window as any).__mockEventSources = MockEventSource.instances;
    (window as any).EventSource = MockEventSource as unknown as EventSource;
  });

  await page.route('**/api/console/lifecycle?**', async (route) => {
    const url = new URL(route.request().url());
    const params = Object.fromEntries(url.searchParams.entries());
    await route.fulfill({
      status: 200,
      body: JSON.stringify(MOCK_PAGE),
      headers: { 'content-type': 'application/json' },
    });
    await page.evaluate((query) => {
      (window as any).__lastLifecycleQuery = query;
    }, params);
  });
});

test.describe('Lifecycle console', () => {
  test('renders timeline shells and verdict cards', async ({ page }) => {
    await page.goto(PAGE_PATH);
    await expect(page.getByRole('heading', { name: 'Lifecycle Console' })).toBeVisible();
    await expect(page.getByLabel('Workspace search')).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Ops Production' })).toBeVisible();
    await expect(page.getByText('Run #9001')).toBeVisible();
    await expect(page.getByText('Promotion posture')).toBeVisible();
    await expect(page.getByText('Lifecycle · production')).toBeVisible();
  });

  test('applies filters and persists them across reloads', async ({ page }) => {
    await page.goto(PAGE_PATH);
    await page.getByLabel('Workspace search').fill('prod');
    await page.getByLabel('Promotion lane').fill('pre-prod');
    await page.getByLabel('Severity').fill('high');
    await expect.poll(async () => page.evaluate(() => window.localStorage.getItem('lifecycle-console.filters.v2'))).toContain(
      '"workspaceSearch":"prod"',
    );
    await page.reload();
    await expect(page.getByLabel('Workspace search')).toHaveValue('prod');
    await expect(page.getByLabel('Promotion lane')).toHaveValue('pre-prod');
    await expect(page.getByLabel('Severity')).toHaveValue('high');
    const recordedQuery = await page.evaluate(() => (window as any).__lastLifecycleQuery);
    expect(recordedQuery.workspace_search).toBe('prod');
    expect(recordedQuery.promotion_lane).toBe('pre-prod');
    expect(recordedQuery.severity).toBe('high');
  });

  test('surfaces delta changes from replay events', async ({ page }) => {
    await page.goto(PAGE_PATH);
    await page.waitForTimeout(100);
    await page.evaluate((delta) => {
      const sources = (window as any).__mockEventSources as any[];
      const source = sources[sources.length - 1];
      source.emit('lifecycle-snapshot', {
        type: 'snapshot',
        emitted_at: new Date().toISOString(),
        cursor: 101,
        page: null,
        delta,
      });
    }, MOCK_DELTA);
    await expect(page.getByText('marketplace.status: eligible → approved')).toBeVisible();
    await expect(page.getByText('Updated')).toBeVisible();
  });

  test('offline mode preserves cached workspaces', async ({ page }) => {
    await page.goto(PAGE_PATH);
    await page.evaluate(() => {
      window.dispatchEvent(new Event('offline'));
    });
    await expect(page.getByText('Offline mode enabled')).toBeVisible();
    await page.reload();
    await expect(page.getByRole('heading', { name: 'Ops Production' })).toBeVisible();
  });
});
