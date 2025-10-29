import { render, screen } from '@testing-library/react';
import LifecycleRunProgress from '../LifecycleRunProgress';
import type { LifecycleRunSnapshot } from '../../../lib/lifecycle-console';

describe('LifecycleRunProgress', () => {
  function buildRun(overrides: Partial<LifecycleRunSnapshot> = {}): LifecycleRunSnapshot {
    const base: LifecycleRunSnapshot = {
      run: {
        id: 42,
        runtime_vm_instance_id: 7,
        playbook: 'deploy',
        playbook_id: null,
        status: 'succeeded',
        automation_payload: null,
        approval_required: false,
        started_at: '2024-01-01T00:00:00.000Z',
        completed_at: '2024-01-01T00:05:30.000Z',
        last_error: null,
        assigned_owner_id: null,
        sla_deadline: null,
        approval_state: 'auto-approved',
        approval_decided_at: '2024-01-01T00:05:35.000Z',
        approval_notes: null,
        metadata: {} as Record<string, unknown>,
        workspace_id: 3,
        workspace_revision_id: 4,
        promotion_gate_context: {} as Record<string, unknown>,
        version: 1,
        updated_at: '2024-01-01T00:06:00.000Z',
        cancelled_at: null,
        cancellation_reason: null,
        failure_reason: null,
      },
      trust: undefined,
      intelligence: [],
      marketplace: undefined,
      duration_seconds: 330,
      retry_attempt: 2,
      retry_limit: 3,
      override_reason: 'manual override',
      artifacts: [
        {
          manifest_digest: 'sha256:abcdef1234567890',
          lane: 'green',
          stage: 'production',
          manifest_tag: 'v1.2.3',
          registry_image: 'registry.example.com/app:v1.2.3',
          build_status: 'succeeded',
          duration_seconds: 125,
        },
      ],
    };
    return { ...base, ...overrides };
  }

  it('renders attempt, override, and artifact analytics chips', () => {
    const run = buildRun();
    render(<LifecycleRunProgress run={run} />);

    expect(screen.getByText('Attempt 2/3')).toBeInTheDocument();
    expect(screen.getByText(/Override: manual override/)).toBeInTheDocument();
    expect(
      screen.getByText('sha256:abcdef1234… (lane:green, stage:production, tag:v1.2.3, image:registry.example.com/app:v1.2.3, build:succeeded, duration:2m 5s)'),
    ).toBeInTheDocument();
  });

  it('falls back to computed duration when analytics missing', () => {
    const run = buildRun({ duration_seconds: null, artifacts: [] });
    render(<LifecycleRunProgress run={run} />);

    expect(screen.getByText(/5m 30s/)).toBeInTheDocument();
    expect(screen.queryByText(/sha256:/)).not.toBeInTheDocument();
  });
});
