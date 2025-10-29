import { fireEvent, render, screen } from '@testing-library/react';
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
    const artifactChip = screen.getByText(/lane:green/);
    expect(artifactChip).toHaveTextContent('sha256:abcde…');
    expect(artifactChip).toHaveTextContent('duration:2m 5s');
  });

  it('falls back to computed duration when analytics missing', () => {
    const run = buildRun({ duration_seconds: null, artifacts: [] });
    render(<LifecycleRunProgress run={run} />);

    expect(screen.getByText(/5m 30s/)).toBeInTheDocument();
    expect(screen.queryByText(/sha256:/)).not.toBeInTheDocument();
  });

  it('renders retry ledger, override actor, promotion verdict, fingerprints, and handles selection', () => {
    const run = buildRun({
      run: {
        ...buildRun().run,
        status: 'failed',
        failure_reason: 'exhausted',
        approval_required: true,
        approval_state: 'pending',
      },
      duration_ms: 125_000,
      duration_seconds: null,
      execution_window: {
        started_at: '2024-01-01T00:00:00.000Z',
        completed_at: '2024-01-01T00:02:05.000Z',
      },
      retry_attempt: null,
      retry_limit: 4,
      retry_count: 5,
      retry_ledger: [
        {
          attempt: 4,
          status: 'failed',
          reason: 'timeout',
          observed_at: '2024-01-01T00:02:00.000Z',
        },
        {
          attempt: 5,
          status: 'succeeded',
          observed_at: '2024-01-01T00:02:05.000Z',
        },
      ],
      override_reason: null,
      manual_override: {
        reason: 'force override',
        actor_email: 'ops@example.com',
        actor_id: 101,
      },
      artifact_fingerprints: [
        {
          manifest_digest: 'sha256:abcdef1234567890',
          fingerprint: 'f47ac10b58cc4372a5670e02b2c3d479',
        },
      ],
      promotion_verdict: {
        verdict_id: 555,
        allowed: false,
        stage: 'production',
        track_name: 'stable',
        track_tier: 'gold',
      },
    });

    const onSelect = jest.fn();
    render(<LifecycleRunProgress run={run} onSelect={onSelect} />);

    expect(screen.getByText(/Attempt\s+–\/4/)).toBeInTheDocument();
    expect(screen.getByText('5 attempts logged')).toBeInTheDocument();
    expect(screen.getByText(/latest #5 · succeeded/)).toBeInTheDocument();
    expect(screen.getByText(/force override · actor:ops@example.com/)).toBeInTheDocument();
    expect(screen.getByText(/verdict #555 · blocked · stage:production · track:stable · tier:gold/)).toBeInTheDocument();
    expect(screen.getByText(/Fingerprints:/)).toHaveTextContent(/sha256:abcde…=f47ac10b58cc4372…/);
    expect(screen.getByText(/Awaiting approval/)).toBeInTheDocument();
    expect(screen.getByText(/Failure: exhausted/)).toBeInTheDocument();
    expect(screen.getByText(/Started.*2m 5s/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /View details/ }));
    expect(onSelect).toHaveBeenCalledWith(run);
  });
});
