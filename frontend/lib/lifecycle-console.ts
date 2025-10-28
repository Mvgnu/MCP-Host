// key: lifecycle-console-ui -> data-contract
export interface LifecycleConsolePage {
  workspaces: LifecycleWorkspaceSnapshot[];
  next_cursor?: number | null;
}

export interface LifecycleWorkspaceSnapshot {
  workspace: RemediationWorkspace;
  active_revision?: LifecycleWorkspaceRevision;
  recent_runs: LifecycleRunSnapshot[];
}

export interface LifecycleWorkspaceRevision {
  revision: RemediationWorkspaceRevision;
  gate_snapshots: WorkspaceValidationSnapshot[];
}

export interface LifecycleRunSnapshot {
  run: RemediationRun;
  trust?: TrustRegistryState;
  intelligence: IntelligenceScoreOverview[];
  marketplace?: MarketplaceReadiness;
}

export interface IntelligenceScoreOverview {
  capability: string;
  backend?: string | null;
  tier?: string | null;
  score: number;
  status: string;
  confidence: number;
  last_observed_at: string;
}

export interface MarketplaceReadiness {
  status: string;
  last_completed_at?: string | null;
}

export interface LifecycleConsoleEventEnvelope {
  type: 'snapshot' | 'heartbeat' | 'error';
  emitted_at: string;
  cursor?: number | null;
  page?: LifecycleConsolePage;
  error?: string;
}

export interface RemediationWorkspace {
  id: number;
  workspace_key: string;
  display_name: string;
  description?: string | null;
  owner_id: number;
  lifecycle_state: string;
  active_revision_id?: number | null;
  metadata?: Record<string, unknown> | null;
  lineage_tags?: string[] | null;
  created_at: string;
  updated_at: string;
  version: number;
}

export interface RemediationWorkspaceRevision {
  id: number;
  workspace_id: number;
  revision_number: number;
  previous_revision_id?: number | null;
  created_by: number;
  plan: Record<string, unknown>;
  schema_status?: string | null;
  schema_errors?: unknown;
  policy_status?: string | null;
  policy_veto_reasons?: unknown;
  simulation_status?: string | null;
  promotion_status?: string | null;
  metadata?: Record<string, unknown> | null;
  lineage_labels?: string[] | null;
  schema_validated_at?: string | null;
  policy_evaluated_at?: string | null;
  simulated_at?: string | null;
  promoted_at?: string | null;
  created_at: string;
  updated_at: string;
  version: number;
}

export interface WorkspaceValidationSnapshot {
  id: number;
  workspace_revision_id: number;
  snapshot_type: string;
  status: string;
  gate_context?: Record<string, unknown> | null;
  notes?: string | null;
  recorded_at: string;
  metadata?: Record<string, unknown> | null;
  created_at: string;
  updated_at: string;
  version: number;
}

export interface RemediationRun {
  id: number;
  runtime_vm_instance_id: number;
  playbook: string;
  playbook_id?: number | null;
  status: string;
  automation_payload?: Record<string, unknown> | null;
  approval_required: boolean;
  started_at: string;
  completed_at?: string | null;
  last_error?: string | null;
  assigned_owner_id?: number | null;
  sla_deadline?: string | null;
  approval_state?: string | null;
  approval_decided_at?: string | null;
  approval_notes?: string | null;
  metadata?: Record<string, unknown> | null;
  workspace_id?: number | null;
  workspace_revision_id?: number | null;
  promotion_gate_context?: Record<string, unknown> | null;
  version: number;
  updated_at: string;
  cancelled_at?: string | null;
  cancellation_reason?: string | null;
  failure_reason?: string | null;
}

export interface TrustRegistryState {
  runtime_vm_instance_id: number;
  attestation_status: string;
  lifecycle_state: string;
  remediation_state?: string | null;
  remediation_attempts: number;
  freshness_deadline?: string | null;
  provenance_ref?: string | null;
  provenance?: Record<string, unknown> | null;
  version: number;
  updated_at: string;
}
