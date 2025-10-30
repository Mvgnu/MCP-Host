use crate::docker;
use crate::extractor::AuthUser;
use axum::{
    extract::{Extension, Path},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row};
use tracing::error;
use uuid::Uuid;

#[derive(Serialize)]
pub struct VectorDb {
    pub id: i32,
    pub name: String,
    pub db_type: String,
    pub url: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// key: vector-dbs-residency-policy
/// Residency policy metadata persisted per managed vector database.
#[derive(Serialize)]
pub struct VectorDbResidencyPolicy {
    pub id: i32,
    pub vector_db_id: i32,
    pub region: String,
    pub data_classification: String,
    pub enforcement_mode: String,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Deserialize)]
pub struct UpsertVectorDbResidencyPolicy {
    pub region: String,
    #[serde(default = "default_data_classification")]
    pub data_classification: String,
    #[serde(default = "default_enforcement_mode")]
    pub enforcement_mode: String,
    #[serde(default = "default_active_flag")]
    pub active: bool,
}

fn default_data_classification() -> String {
    "general".into()
}

fn default_enforcement_mode() -> String {
    "monitor".into()
}

fn default_active_flag() -> bool {
    true
}

/// key: vector-dbs-attachment
/// Attachment record ensuring residency + BYOK posture for dependent services.
#[derive(Serialize)]
pub struct VectorDbAttachment {
    pub id: Uuid,
    pub vector_db_id: i32,
    pub attachment_type: String,
    pub attachment_ref: Uuid,
    pub residency_policy_id: i32,
    pub provider_key_binding_id: Uuid,
    pub provider_key_id: Uuid,
    pub provider_key_rotation_due_at: Option<DateTime<Utc>>,
    pub attached_at: DateTime<Utc>,
    pub detached_at: Option<DateTime<Utc>>,
    pub detached_reason: Option<String>,
    pub metadata: Value,
}

#[derive(Deserialize)]
pub struct CreateVectorDbAttachment {
    pub attachment_type: String,
    pub attachment_ref: Uuid,
    pub residency_policy_id: i32,
    pub provider_key_binding_id: Uuid,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Deserialize)]
pub struct DetachVectorDbAttachment {
    #[serde(default)]
    pub reason: Option<String>,
}

/// key: vector-dbs-incident-log
/// Compliance incidents recorded against federated vector DB attachments.
#[derive(Serialize)]
pub struct VectorDbIncidentLog {
    pub id: Uuid,
    pub vector_db_id: i32,
    pub attachment_id: Option<Uuid>,
    pub incident_type: String,
    pub severity: String,
    pub occurred_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub summary: Option<String>,
    pub notes: Value,
}

#[derive(Deserialize)]
pub struct CreateVectorDbIncident {
    pub incident_type: String,
    #[serde(default = "default_incident_severity")]
    pub severity: String,
    pub attachment_id: Option<Uuid>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub notes: Value,
}

#[derive(Deserialize)]
pub struct ResolveVectorDbIncident {
    #[serde(default)]
    pub resolution_summary: Option<String>,
    #[serde(default)]
    pub resolution_notes: Option<Value>,
}

fn default_incident_severity() -> String {
    "medium".into()
}

#[derive(Deserialize)]
pub struct CreateVectorDb {
    pub name: String,
    #[serde(default = "default_db_type")]
    pub db_type: String,
}

fn default_db_type() -> String {
    "chroma".into()
}

async fn ensure_vector_db_owner(
    pool: &PgPool,
    vector_db_id: i32,
    user_id: i32,
) -> Result<(), (StatusCode, String)> {
    let owner: Option<i32> = sqlx::query_scalar("SELECT owner_id FROM vector_dbs WHERE id = $1")
        .bind(vector_db_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| {
            error!(?e, vector_db_id, "DB error verifying vector db owner");
            (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
        })?;

    match owner {
        Some(db_owner) if db_owner == user_id => Ok(()),
        Some(_) => Err((StatusCode::FORBIDDEN, "Vector DB ownership mismatch".into())),
        None => Err((StatusCode::NOT_FOUND, "Vector DB not found".into())),
    }
}

pub async fn list_vector_dbs(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
) -> Result<Json<Vec<VectorDb>>, (StatusCode, String)> {
    let rows = sqlx::query(
        "SELECT id, name, db_type, url, created_at FROM vector_dbs WHERE owner_id = $1 ORDER BY id",
    )
    .bind(user_id)
    .fetch_all(&pool)
    .await
    .map_err(|e| {
        error!(?e, "DB error listing vector dbs");
        (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
    })?;
    let list = rows
        .into_iter()
        .map(|r| VectorDb {
            id: r.get("id"),
            name: r.get("name"),
            db_type: r.get("db_type"),
            url: r.try_get("url").ok(),
            created_at: r.get("created_at"),
        })
        .collect();
    Ok(Json(list))
}

pub async fn create_vector_db(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Json(payload): Json<CreateVectorDb>,
) -> Result<Json<VectorDb>, (StatusCode, String)> {
    let rec = sqlx::query(
        "INSERT INTO vector_dbs (owner_id, name, db_type) VALUES ($1,$2,$3) RETURNING id, created_at"
    )
    .bind(user_id)
    .bind(&payload.name)
    .bind(&payload.db_type)
    .fetch_one(&pool)
    .await
    .map_err(|e| {
        error!(?e, "DB error creating vector db");
        (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
    })?;
    let id: i32 = rec.get("id");
    let created_at: chrono::DateTime<chrono::Utc> = rec.get("created_at");
    docker::spawn_vector_db_task(id, payload.db_type.clone(), pool.clone());
    Ok(Json(VectorDb {
        id,
        name: payload.name,
        db_type: payload.db_type,
        url: None,
        created_at,
    }))
}

pub async fn delete_vector_db(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(id): Path<i32>,
) -> Result<StatusCode, (StatusCode, String)> {
    ensure_vector_db_owner(&pool, id, user_id).await?;
    docker::delete_vector_db_task(id, pool.clone());
    Ok(StatusCode::NO_CONTENT)
}

pub async fn upsert_vector_db_residency_policy(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(id): Path<i32>,
    Json(payload): Json<UpsertVectorDbResidencyPolicy>,
) -> Result<Json<VectorDbResidencyPolicy>, (StatusCode, String)> {
    ensure_vector_db_owner(&pool, id, user_id).await?;

    let UpsertVectorDbResidencyPolicy {
        region,
        data_classification,
        enforcement_mode,
        active,
    } = payload;

    let row = sqlx::query(
        r#"
        INSERT INTO vector_db_residency_policies(vector_db_id, region, data_classification, enforcement_mode, active)
        VALUES ($1,$2,$3,$4,$5)
        ON CONFLICT (vector_db_id, region)
        DO UPDATE SET
            data_classification = EXCLUDED.data_classification,
            enforcement_mode = EXCLUDED.enforcement_mode,
            active = EXCLUDED.active,
            updated_at = NOW()
        RETURNING id, vector_db_id, region, data_classification, enforcement_mode, active, created_at, updated_at
        "#,
    )
    .bind(id)
    .bind(&region)
    .bind(&data_classification)
    .bind(&enforcement_mode)
    .bind(active)
    .fetch_one(&pool)
    .await
    .map_err(|e| {
        error!(?e, vector_db_id = id, region, "DB error upserting residency policy");
        (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
    })?;

    Ok(Json(VectorDbResidencyPolicy {
        id: row.get("id"),
        vector_db_id: row.get("vector_db_id"),
        region: row.get("region"),
        data_classification: row.get("data_classification"),
        enforcement_mode: row.get("enforcement_mode"),
        active: row.get("active"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }))
}

pub async fn list_vector_db_residency_policies(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(id): Path<i32>,
) -> Result<Json<Vec<VectorDbResidencyPolicy>>, (StatusCode, String)> {
    ensure_vector_db_owner(&pool, id, user_id).await?;

    let rows = sqlx::query(
        "SELECT id, vector_db_id, region, data_classification, enforcement_mode, active, created_at, updated_at FROM vector_db_residency_policies WHERE vector_db_id = $1 ORDER BY region",
    )
    .bind(id)
    .fetch_all(&pool)
    .await
    .map_err(|e| {
        error!(?e, vector_db_id = id, "DB error listing residency policies");
        (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
    })?;

    Ok(Json(
        rows.into_iter()
            .map(|row| VectorDbResidencyPolicy {
                id: row.get("id"),
                vector_db_id: row.get("vector_db_id"),
                region: row.get("region"),
                data_classification: row.get("data_classification"),
                enforcement_mode: row.get("enforcement_mode"),
                active: row.get("active"),
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
            })
            .collect(),
    ))
}

pub async fn create_vector_db_attachment(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(id): Path<i32>,
    Json(payload): Json<CreateVectorDbAttachment>,
) -> Result<Json<VectorDbAttachment>, (StatusCode, String)> {
    ensure_vector_db_owner(&pool, id, user_id).await?;

    let CreateVectorDbAttachment {
        attachment_type,
        attachment_ref,
        residency_policy_id,
        provider_key_binding_id,
        metadata,
    } = payload;

    let policy = sqlx::query(
        "SELECT id, active FROM vector_db_residency_policies WHERE id = $1 AND vector_db_id = $2",
    )
    .bind(residency_policy_id)
    .bind(id)
    .fetch_optional(&pool)
    .await
    .map_err(|e| {
        error!(
            ?e,
            policy_id = residency_policy_id,
            vector_db_id = id,
            "DB error loading residency policy"
        );
        (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
    })?;

    match policy {
        Some(row) => {
            let active: bool = row.get("active");
            if !active {
                return Err((StatusCode::CONFLICT, "Residency policy is inactive".into()));
            }
        }
        None => {
            return Err((StatusCode::NOT_FOUND, "Residency policy not found".into()));
        }
    }

    let binding = sqlx::query(
        "SELECT binding_type, provider_key_id FROM provider_key_bindings WHERE id = $1 AND revoked_at IS NULL",
    )
    .bind(provider_key_binding_id)
    .fetch_optional(&pool)
    .await
    .map_err(|e| {
        error!(
            ?e,
            binding_id = %provider_key_binding_id,
            "DB error loading provider key binding"
        );
        (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
    })?;

    let provider_key_id = match binding {
        Some(row) => {
            let binding_type: String = row.get("binding_type");
            if binding_type != "vector_db" {
                return Err((
                    StatusCode::CONFLICT,
                    "Provider key binding must target vector_db attachments".into(),
                ));
            }
            row.get("provider_key_id")
        }
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                "Active provider key binding not found".into(),
            ));
        }
    };

    let rotation_due_at: Option<DateTime<Utc>> = sqlx::query_scalar(
        "SELECT rotation_due_at FROM provider_keys WHERE id = $1",
    )
    .bind(provider_key_id)
    .fetch_one(&pool)
    .await
    .map_err(|e| {
        error!(?e, provider_key_id = %provider_key_id, "DB error loading provider key rotation due");
        (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
    })?;

    let attachment_id = Uuid::new_v4();
    let row = sqlx::query(
        r#"
        INSERT INTO vector_db_attachments(id, vector_db_id, attachment_type, attachment_ref, residency_policy_id, provider_key_binding_id, metadata)
        VALUES($1,$2,$3,$4,$5,$6,$7)
        RETURNING id, vector_db_id, attachment_type, attachment_ref, residency_policy_id, provider_key_binding_id, attached_at, detached_at, detached_reason, metadata
        "#,
    )
    .bind(attachment_id)
    .bind(id)
    .bind(&attachment_type)
    .bind(attachment_ref)
    .bind(residency_policy_id)
    .bind(provider_key_binding_id)
    .bind(&metadata)
    .fetch_one(&pool)
    .await
    .map_err(|e| {
        error!(
            ?e,
            vector_db_id = id,
            attachment_id = %attachment_id,
            "DB error creating vector db attachment"
        );
        (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
    })?;

    Ok(Json(VectorDbAttachment {
        id: row.get("id"),
        vector_db_id: row.get("vector_db_id"),
        attachment_type: row.get("attachment_type"),
        attachment_ref: row.get("attachment_ref"),
        residency_policy_id: row.get("residency_policy_id"),
        provider_key_binding_id: row.get("provider_key_binding_id"),
        provider_key_id,
        provider_key_rotation_due_at: rotation_due_at,
        attached_at: row.get("attached_at"),
        detached_at: row.try_get("detached_at").ok(),
        detached_reason: row.try_get("detached_reason").ok(),
        metadata: row.get("metadata"),
    }))
}

pub async fn list_vector_db_attachments(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(id): Path<i32>,
) -> Result<Json<Vec<VectorDbAttachment>>, (StatusCode, String)> {
    ensure_vector_db_owner(&pool, id, user_id).await?;

    let rows = sqlx::query(
        r#"SELECT a.id,
                  a.vector_db_id,
                  a.attachment_type,
                  a.attachment_ref,
                  a.residency_policy_id,
                  a.provider_key_binding_id,
                  b.provider_key_id,
                  k.rotation_due_at,
                  a.attached_at,
                  a.detached_at,
                  a.detached_reason,
                  a.metadata
           FROM vector_db_attachments a
           INNER JOIN provider_key_bindings b ON a.provider_key_binding_id = b.id
           INNER JOIN provider_keys k ON b.provider_key_id = k.id
           WHERE a.vector_db_id = $1
           ORDER BY a.attached_at DESC"#,
    )
    .bind(id)
    .fetch_all(&pool)
    .await
    .map_err(|e| {
        error!(
            ?e,
            vector_db_id = id,
            "DB error listing vector db attachments"
        );
        (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
    })?;

    Ok(Json(
        rows.into_iter()
            .map(|row| VectorDbAttachment {
                id: row.get("id"),
                vector_db_id: row.get("vector_db_id"),
                attachment_type: row.get("attachment_type"),
                attachment_ref: row.get("attachment_ref"),
                residency_policy_id: row.get("residency_policy_id"),
                provider_key_binding_id: row.get("provider_key_binding_id"),
                provider_key_id: row.get("provider_key_id"),
                provider_key_rotation_due_at: row.try_get("rotation_due_at").ok(),
                attached_at: row.get("attached_at"),
                detached_at: row.try_get("detached_at").ok(),
                detached_reason: row.try_get("detached_reason").ok(),
                metadata: row.get("metadata"),
            })
            .collect(),
    ))
}

pub async fn detach_vector_db_attachment(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path((id, attachment_id)): Path<(i32, Uuid)>,
    Json(payload): Json<DetachVectorDbAttachment>,
) -> Result<Json<VectorDbAttachment>, (StatusCode, String)> {
    ensure_vector_db_owner(&pool, id, user_id).await?;

    let existing = sqlx::query(
        "SELECT detached_at FROM vector_db_attachments WHERE id = $1 AND vector_db_id = $2",
    )
    .bind(attachment_id)
    .bind(id)
    .fetch_optional(&pool)
    .await
    .map_err(|e| {
        error!(?e, attachment_id = %attachment_id, vector_db_id = id, "DB error loading attachment state");
        (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
    })?;

    match existing {
        None => return Err((StatusCode::NOT_FOUND, "Attachment not found".into())),
        Some(row) => {
            let already_detached: Option<DateTime<Utc>> = row.try_get("detached_at").ok();
            if already_detached.is_some() {
                return Err((StatusCode::CONFLICT, "Attachment already detached".into()));
            }
        }
    }

    let reason = payload
        .reason
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "Detached by operator".into());

    let row = sqlx::query(
        r#"UPDATE vector_db_attachments AS a
           SET detached_at = NOW(),
               detached_reason = $3
           FROM provider_key_bindings b
           INNER JOIN provider_keys k ON b.provider_key_id = k.id
           WHERE a.id = $2
             AND a.vector_db_id = $1
             AND a.provider_key_binding_id = b.id
           RETURNING a.id,
                     a.vector_db_id,
                     a.attachment_type,
                     a.attachment_ref,
                     a.residency_policy_id,
                     a.provider_key_binding_id,
                     b.provider_key_id,
                     k.rotation_due_at,
                     a.attached_at,
                     a.detached_at,
                     a.detached_reason,
                     a.metadata"#,
    )
    .bind(id)
    .bind(attachment_id)
    .bind(&reason)
    .fetch_one(&pool)
    .await
    .map_err(|e| {
        error!(
            ?e,
            attachment_id = %attachment_id,
            vector_db_id = id,
            "DB error detaching vector db attachment"
        );
        (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
    })?;

    Ok(Json(VectorDbAttachment {
        id: row.get("id"),
        vector_db_id: row.get("vector_db_id"),
        attachment_type: row.get("attachment_type"),
        attachment_ref: row.get("attachment_ref"),
        residency_policy_id: row.get("residency_policy_id"),
        provider_key_binding_id: row.get("provider_key_binding_id"),
        provider_key_id: row.get("provider_key_id"),
        provider_key_rotation_due_at: row.try_get("rotation_due_at").ok(),
        attached_at: row.get("attached_at"),
        detached_at: row.try_get("detached_at").ok(),
        detached_reason: row.try_get("detached_reason").ok(),
        metadata: row.get("metadata"),
    }))
}

pub async fn log_vector_db_incident(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(id): Path<i32>,
    Json(payload): Json<CreateVectorDbIncident>,
) -> Result<Json<VectorDbIncidentLog>, (StatusCode, String)> {
    ensure_vector_db_owner(&pool, id, user_id).await?;

    let CreateVectorDbIncident {
        incident_type,
        severity,
        attachment_id,
        summary,
        notes,
    } = payload;

    if let Some(attachment_id) = attachment_id {
        let owned: Option<i32> = sqlx::query_scalar(
            "SELECT 1 FROM vector_db_attachments WHERE id = $1 AND vector_db_id = $2",
        )
        .bind(attachment_id)
        .bind(id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| {
            error!(
                ?e,
                attachment_id = %attachment_id,
                vector_db_id = id,
                "DB error verifying attachment ownership"
            );
            (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
        })?;

        if owned.is_none() {
            return Err((
                StatusCode::NOT_FOUND,
                "Attachment not found for vector db".into(),
            ));
        }
    }

    let incident_id = Uuid::new_v4();
    let row = sqlx::query(
        r#"
        INSERT INTO vector_db_incident_logs(id, vector_db_id, attachment_id, incident_type, severity, summary, notes)
        VALUES($1,$2,$3,$4,$5,$6,$7)
        RETURNING id, vector_db_id, attachment_id, incident_type, severity, occurred_at, resolved_at, summary, notes
        "#,
    )
    .bind(incident_id)
    .bind(id)
    .bind(attachment_id)
    .bind(&incident_type)
    .bind(&severity)
    .bind(&summary)
    .bind(&notes)
    .fetch_one(&pool)
    .await
    .map_err(|e| {
        error!(
            ?e,
            incident_id = %incident_id,
            vector_db_id = id,
            "DB error creating vector db incident"
        );
        (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
    })?;

    Ok(Json(VectorDbIncidentLog {
        id: row.get("id"),
        vector_db_id: row.get("vector_db_id"),
        attachment_id: row.try_get("attachment_id").ok(),
        incident_type: row.get("incident_type"),
        severity: row.get("severity"),
        occurred_at: row.get("occurred_at"),
        resolved_at: row.try_get("resolved_at").ok(),
        summary: row.try_get("summary").ok(),
        notes: row.get("notes"),
    }))
}

pub async fn list_vector_db_incidents(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(id): Path<i32>,
) -> Result<Json<Vec<VectorDbIncidentLog>>, (StatusCode, String)> {
    ensure_vector_db_owner(&pool, id, user_id).await?;

    let rows = sqlx::query(
        "SELECT id, vector_db_id, attachment_id, incident_type, severity, occurred_at, resolved_at, summary, notes FROM vector_db_incident_logs WHERE vector_db_id = $1 ORDER BY occurred_at DESC",
    )
    .bind(id)
    .fetch_all(&pool)
    .await
    .map_err(|e| {
        error!(?e, vector_db_id = id, "DB error listing vector db incidents");
        (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
    })?;

    Ok(Json(
        rows.into_iter()
            .map(|row| VectorDbIncidentLog {
                id: row.get("id"),
                vector_db_id: row.get("vector_db_id"),
                attachment_id: row.try_get("attachment_id").ok(),
                incident_type: row.get("incident_type"),
                severity: row.get("severity"),
                occurred_at: row.get("occurred_at"),
                resolved_at: row.try_get("resolved_at").ok(),
                summary: row.try_get("summary").ok(),
                notes: row.get("notes"),
            })
            .collect(),
    ))
}

pub async fn resolve_vector_db_incident(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path((id, incident_id)): Path<(i32, Uuid)>,
    Json(payload): Json<ResolveVectorDbIncident>,
) -> Result<Json<VectorDbIncidentLog>, (StatusCode, String)> {
    ensure_vector_db_owner(&pool, id, user_id).await?;

    let existing = sqlx::query(
        "SELECT resolved_at FROM vector_db_incident_logs WHERE id = $1 AND vector_db_id = $2",
    )
    .bind(incident_id)
    .bind(id)
    .fetch_optional(&pool)
    .await
    .map_err(|e| {
        error!(?e, incident_id = %incident_id, vector_db_id = id, "DB error loading incident state");
        (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
    })?;

    match existing {
        None => return Err((StatusCode::NOT_FOUND, "Incident not found".into())),
        Some(row) => {
            let resolved: Option<DateTime<Utc>> = row.try_get("resolved_at").ok();
            if resolved.is_some() {
                return Err((StatusCode::CONFLICT, "Incident already resolved".into()));
            }
        }
    }

    let row = sqlx::query(
        r#"UPDATE vector_db_incident_logs
           SET resolved_at = NOW(),
               summary = COALESCE($3, summary),
               notes = CASE WHEN $4::jsonb IS NULL THEN notes ELSE $4::jsonb END
           WHERE id = $2 AND vector_db_id = $1
           RETURNING id,
                     vector_db_id,
                     attachment_id,
                     incident_type,
                     severity,
                     occurred_at,
                     resolved_at,
                     summary,
                     notes"#,
    )
    .bind(id)
    .bind(incident_id)
    .bind(payload.resolution_summary.as_ref())
    .bind(payload.resolution_notes.as_ref())
    .fetch_one(&pool)
    .await
    .map_err(|e| {
        error!(?e, incident_id = %incident_id, vector_db_id = id, "DB error resolving incident");
        (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
    })?;

    Ok(Json(VectorDbIncidentLog {
        id: row.get("id"),
        vector_db_id: row.get("vector_db_id"),
        attachment_id: row.try_get("attachment_id").ok(),
        incident_type: row.get("incident_type"),
        severity: row.get("severity"),
        occurred_at: row.get("occurred_at"),
        resolved_at: row.try_get("resolved_at").ok(),
        summary: row.try_get("summary").ok(),
        notes: row.get("notes"),
    }))
}
