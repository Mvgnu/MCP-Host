use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "governance_workflow_kind", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum GovernanceWorkflowKind {
    Promotion,
    Rollback,
    CredentialRotation,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "governance_run_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum GovernanceRunStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "governance_step_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum GovernanceStepStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceWorkflowStepInput {
    pub action: String,
    #[serde(default)]
    pub config: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateGovernanceWorkflow {
    pub name: String,
    pub workflow_type: GovernanceWorkflowKind,
    pub tier: String,
    #[serde(default)]
    pub steps: Vec<GovernanceWorkflowStepInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct GovernanceWorkflow {
    pub id: i32,
    pub owner_id: i32,
    pub name: String,
    pub workflow_type: GovernanceWorkflowKind,
    pub tier: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct GovernanceWorkflowStep {
    pub id: i32,
    pub workflow_id: i32,
    pub position: i32,
    pub action: String,
    pub config: Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartWorkflowRunRequest {
    pub target_manifest_digest: Option<String>,
    pub target_artifact_run_id: Option<i32>,
    pub notes: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunStatusUpdateRequest {
    pub status: GovernanceRunStatus,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct GovernanceStepRunDetail {
    pub id: i64,
    pub step_id: Option<i32>,
    pub action: Option<String>,
    pub status: GovernanceStepStatus,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct GovernanceAuditLogEntry {
    pub id: i64,
    pub event_type: String,
    pub details: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub actor_id: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceRunDetail {
    pub id: i64,
    pub workflow_id: i32,
    pub status: GovernanceRunStatus,
    pub notes: Vec<String>,
    pub target_manifest_digest: Option<String>,
    pub target_tier: String,
    pub initiated_by: Option<i32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub steps: Vec<GovernanceStepRunDetail>,
    pub audit_log: Vec<GovernanceAuditLogEntry>,
}
