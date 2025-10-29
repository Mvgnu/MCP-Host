import { useCallback, useMemo, useState } from 'react';
import type { Dispatch, SetStateAction } from 'react';
import type {
  LifecyclePromotionPostureDelta,
  LifecycleRunDelta,
  LifecycleRunSnapshot,
  LifecycleWorkspaceSnapshot,
} from './lifecycle-console';

// key: lifecycle-console-ui -> action-client
export type WorkspaceMap = Record<number, LifecycleWorkspaceSnapshot>;
export type RunDeltaMap = Record<number, LifecycleRunDelta>;
export type PromotionDeltaMap = Record<number, LifecyclePromotionPostureDelta>;

export interface LifecycleActionState {
  pendingPromotions: Record<number, boolean>;
  pendingApprovals: Record<number, boolean>;
  actionError: string | null;
  clearActionError: () => void;
  applyPromotionStatus: (
    workspaceId: number,
    revisionId: number,
    status: 'approved' | 'rejected' | 'completed',
    options?: { notes?: string[] },
  ) => Promise<void>;
  updateRunApproval: (
    workspaceId: number,
    runId: number,
    newState: 'approved' | 'rejected',
    options?: { notes?: string },
  ) => Promise<void>;
}

export interface LifecycleActionOptions {
  workspaces: WorkspaceMap;
  setWorkspaces: Dispatch<SetStateAction<WorkspaceMap>>;
  setRunDeltas: Dispatch<SetStateAction<RunDeltaMap>>;
  fetchImpl?: typeof fetch;
}

function nowIso() {
  return new Date().toISOString();
}

export function optimisticPromotionUpdate(
  snapshot: LifecycleWorkspaceSnapshot,
  status: 'approved' | 'rejected' | 'completed',
  emittedAt = nowIso(),
): LifecycleWorkspaceSnapshot {
  if (!snapshot.active_revision) {
    return snapshot;
  }
  const promotedWorkspace =
    status === 'completed'
      ? {
          ...snapshot.workspace,
          lifecycle_state: 'promoted',
          updated_at: emittedAt,
        }
      : snapshot.workspace;
  return {
    ...snapshot,
    workspace: promotedWorkspace,
    active_revision: {
      ...snapshot.active_revision,
      revision: {
        ...snapshot.active_revision.revision,
        promotion_status: status,
        updated_at: emittedAt,
      },
    },
  };
}

export function optimisticRunApprovalUpdate(
  snapshot: LifecycleWorkspaceSnapshot,
  runId: number,
  newState: 'approved' | 'rejected',
  emittedAt = nowIso(),
  notes?: string,
): LifecycleWorkspaceSnapshot {
  const updatedRuns = snapshot.recent_runs.map((entry) => {
    if (entry.run.id !== runId) {
      return entry;
    }
    const updatedRun: LifecycleRunSnapshot = {
      ...entry,
      run: {
        ...entry.run,
        approval_state: newState,
        approval_notes: notes ?? entry.run.approval_notes ?? null,
        approval_decided_at: emittedAt,
        updated_at: emittedAt,
      },
    };
    return updatedRun;
  });
  return {
    ...snapshot,
    recent_runs: updatedRuns,
  };
}

export function useLifecycleActions({
  workspaces,
  setWorkspaces,
  setRunDeltas,
  fetchImpl = fetch,
}: LifecycleActionOptions): LifecycleActionState {
  const [pendingPromotions, setPendingPromotions] = useState<Record<number, boolean>>({});
  const [pendingApprovals, setPendingApprovals] = useState<Record<number, boolean>>({});
  const [actionError, setActionError] = useState<string | null>(null);

  const clearActionError = useCallback(() => setActionError(null), []);

  const applyPromotionStatus = useCallback<LifecycleActionState['applyPromotionStatus']>(
    async (workspaceId, revisionId, status, options = {}) => {
    const snapshot = workspaces[workspaceId];
    if (!snapshot || !snapshot.active_revision) {
      setActionError('Workspace revision unavailable for promotion update');
      return;
    }
    const expectedWorkspaceVersion = snapshot.workspace.version;
    const expectedRevisionVersion = snapshot.active_revision.revision.version;

    let previousSnapshot: LifecycleWorkspaceSnapshot | undefined;
    setPendingPromotions((current) => ({ ...current, [workspaceId]: true }));
    setWorkspaces((current) => {
      const currentSnapshot = current[workspaceId];
      if (!currentSnapshot) {
        return current;
      }
      previousSnapshot = currentSnapshot;
      return {
        ...current,
        [workspaceId]: optimisticPromotionUpdate(currentSnapshot, status),
      };
    });
    try {
      const response = await fetchImpl(
        `/api/trust/remediation/workspaces/${workspaceId}/revisions/${revisionId}/promotion`,
        {
          method: 'POST',
          credentials: 'include',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            promotion_status: status,
            notes: options.notes ?? ['console-action'],
            gate_context: {},
            expected_workspace_version: expectedWorkspaceVersion,
            expected_revision_version: expectedRevisionVersion,
          }),
        },
      );
      if (!response.ok) {
        const text = await response.text();
        throw new Error(text || `Failed to update promotion (${response.status})`);
      }
      setActionError(null);
    } catch (err) {
      console.error('Failed to apply promotion status', err);
      if (previousSnapshot) {
        const rollback = previousSnapshot;
        setWorkspaces((current) => ({ ...current, [workspaceId]: rollback }));
      }
      setActionError(err instanceof Error ? err.message : 'Failed to update promotion');
    } finally {
      setPendingPromotions((current) => {
        const next = { ...current };
        delete next[workspaceId];
        return next;
      });
    }
  }, [fetchImpl, setWorkspaces, workspaces]);

  const updateRunApproval = useCallback<LifecycleActionState['updateRunApproval']>(
    async (workspaceId, runId, newState, options = {}) => {
    const snapshot = workspaces[workspaceId];
    if (!snapshot) {
      setActionError('Workspace missing when updating approval state');
      return;
    }
    const targetRun = snapshot.recent_runs.find((entry) => entry.run.id === runId);
    if (!targetRun) {
      setActionError('Remediation run not found for approval update');
      return;
    }
    const expectedVersion = targetRun.run.version;

    let previousSnapshot: LifecycleWorkspaceSnapshot | undefined;
    setPendingApprovals((current) => ({ ...current, [runId]: true }));
    setWorkspaces((current) => {
      const currentSnapshot = current[workspaceId];
      if (!currentSnapshot) {
        return current;
      }
      previousSnapshot = currentSnapshot;
      return {
        ...current,
        [workspaceId]: optimisticRunApprovalUpdate(
          currentSnapshot,
          runId,
          newState,
          undefined,
          options.notes,
        ),
      };
    });
    setRunDeltas((current) => {
      const next = { ...current };
      delete next[runId];
      return next;
    });

    try {
      const response = await fetchImpl(`/api/trust/remediation/runs/${runId}/approval`, {
        method: 'POST',
        credentials: 'include',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          new_state: newState,
          approval_notes: options.notes,
          expected_version: expectedVersion,
        }),
      });
      if (!response.ok) {
        const text = await response.text();
        throw new Error(text || `Failed to update run approval (${response.status})`);
      }
      setActionError(null);
    } catch (err) {
      console.error('Failed to update remediation run approval', err);
      if (previousSnapshot) {
        const rollback = previousSnapshot;
        setWorkspaces((current) => ({ ...current, [workspaceId]: rollback }));
      }
      setActionError(err instanceof Error ? err.message : 'Failed to update remediation run');
    } finally {
      setPendingApprovals((current) => {
        const next = { ...current };
        delete next[runId];
        return next;
      });
    }
  }, [fetchImpl, setRunDeltas, setWorkspaces, workspaces]);

  return useMemo(
    () => ({
      pendingPromotions,
      pendingApprovals,
      actionError,
      clearActionError,
      applyPromotionStatus,
      updateRunApproval,
    }),
    [
      actionError,
      applyPromotionStatus,
      clearActionError,
      pendingApprovals,
      pendingPromotions,
      updateRunApproval,
    ],
  );
}
