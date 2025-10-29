import { optimisticPromotionUpdate, optimisticRunApprovalUpdate } from './lifecycle-actions';
import type { LifecycleWorkspaceSnapshot } from './lifecycle-console';

describe('lifecycle actions optimistic helpers', () => {
  const baseSnapshot: LifecycleWorkspaceSnapshot = {
    workspace: {
      id: 42,
      workspace_key: 'demo',
      display_name: 'Demo Workspace',
      description: null,
      owner_id: 7,
      lifecycle_state: 'active',
      active_revision_id: 101,
      metadata: {},
      lineage_tags: [],
      created_at: '2024-01-01T00:00:00.000Z',
      updated_at: '2024-01-01T00:00:00.000Z',
      version: 3,
    },
    active_revision: {
      revision: {
        id: 101,
        workspace_id: 42,
        revision_number: 3,
        previous_revision_id: 100,
        created_by: 9,
        plan: {},
        schema_status: 'passed',
        schema_errors: [],
        policy_status: 'approved',
        policy_veto_reasons: [],
        simulation_status: 'complete',
        promotion_status: 'pending',
        metadata: {},
        lineage_labels: [],
        schema_validated_at: '2024-01-01T00:00:00.000Z',
        policy_evaluated_at: '2024-01-01T00:00:00.000Z',
        simulated_at: '2024-01-01T00:00:00.000Z',
        promoted_at: null,
        created_at: '2024-01-01T00:00:00.000Z',
        updated_at: '2024-01-01T00:00:00.000Z',
        version: 8,
      },
      gate_snapshots: [],
    },
    recent_runs: [
      {
        run: {
          id: 501,
          runtime_vm_instance_id: 999,
          playbook: 'restart',
          playbook_id: 12,
          status: 'pending',
          automation_payload: {},
          approval_required: true,
          started_at: '2024-01-01T00:00:00.000Z',
          completed_at: null,
          last_error: null,
          assigned_owner_id: null,
          sla_deadline: null,
          approval_state: null,
          approval_decided_at: null,
          approval_notes: null,
          metadata: {},
          workspace_id: 42,
          workspace_revision_id: 101,
          promotion_gate_context: {},
          version: 1,
          updated_at: '2024-01-01T00:00:00.000Z',
          cancelled_at: null,
          cancellation_reason: null,
          failure_reason: null,
        },
        trust: undefined,
        intelligence: [],
        marketplace: undefined,
        provider_key_posture: undefined,
      },
    ],
    promotion_runs: [],
    promotion_postures: [],
  };

  it('updates revision promotion status and leaves workspace state when not completed', () => {
    const result = optimisticPromotionUpdate(baseSnapshot, 'approved', '2024-02-01T00:00:00.000Z');
    expect(result.active_revision?.revision.promotion_status).toBe('approved');
    expect(result.workspace.lifecycle_state).toBe('active');
    expect(result.active_revision?.revision.updated_at).toBe('2024-02-01T00:00:00.000Z');
  });

  it('updates workspace lifecycle when promotion is completed', () => {
    const result = optimisticPromotionUpdate(baseSnapshot, 'completed', '2024-03-01T00:00:00.000Z');
    expect(result.active_revision?.revision.promotion_status).toBe('completed');
    expect(result.workspace.lifecycle_state).toBe('promoted');
    expect(result.workspace.updated_at).toBe('2024-03-01T00:00:00.000Z');
  });

  it('marks remediation run approval with decision metadata', () => {
    const result = optimisticRunApprovalUpdate(
      baseSnapshot,
      501,
      'approved',
      '2024-04-01T00:00:00.000Z',
      'console-approval',
    );
    const run = result.recent_runs[0];
    expect(run.run.approval_state).toBe('approved');
    expect(run.run.approval_decided_at).toBe('2024-04-01T00:00:00.000Z');
    expect(run.run.approval_notes).toBe('console-approval');
  });
});
