// key: organizations-api -> self-service-onboarding
use crate::error::{AppError, AppResult};
use crate::extractor::AuthUser;
use axum::extract::Path;
use axum::{
    routing::{get, post},
    Extension, Json, Router,
};
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(serde::Deserialize)]
pub struct NewOrg {
    pub name: String,
}

#[derive(serde::Serialize)]
pub struct OrgInfo {
    pub id: i32,
    pub name: String,
}

pub fn routes() -> Router {
    Router::new()
        .route("/api/orgs", get(list_orgs).post(create_org))
        .route("/api/orgs/:id/members", post(add_member))
        .route(
            "/api/orgs/:id/invitations",
            get(list_invitations).post(create_invitation),
        )
        .route(
            "/api/orgs/invitations/:token/accept",
            post(accept_invitation),
        )
}

pub async fn list_orgs(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
) -> AppResult<Json<Vec<OrgInfo>>> {
    let rows = sqlx::query(
        "SELECT o.id, o.name FROM organizations o \
        JOIN organization_members m ON m.organization_id = o.id \
        WHERE m.user_id = $1",
    )
    .bind(user_id)
    .fetch_all(&pool)
    .await
    .map_err(|e| {
        tracing::error!(?e, "DB error listing orgs");
        AppError::Db(e)
    })?;
    let orgs = rows
        .into_iter()
        .map(|r| OrgInfo {
            id: r.get("id"),
            name: r.get("name"),
        })
        .collect();
    Ok(Json(orgs))
}

pub async fn create_org(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Json(payload): Json<NewOrg>,
) -> AppResult<Json<OrgInfo>> {
    if payload.name.trim().is_empty() {
        return Err(AppError::BadRequest("Name required".into()));
    }
    let rec =
        sqlx::query("INSERT INTO organizations (name, owner_id) VALUES ($1, $2) RETURNING id")
            .bind(&payload.name)
            .bind(user_id)
            .fetch_one(&pool)
            .await
            .map_err(|e| {
                tracing::error!(?e, "DB error creating org");
                AppError::Db(e)
            })?;
    let id: i32 = rec.get("id");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role) VALUES ($1, $2, 'owner')"
    )
    .bind(id)
    .bind(user_id)
    .execute(&pool)
    .await
    .map_err(|e| {
        tracing::error!(?e, "DB error adding owner to org");
        AppError::Db(e)
    })?;
    Ok(Json(OrgInfo {
        id,
        name: payload.name,
    }))
}

#[derive(serde::Deserialize)]
pub struct AddMemberPayload {
    pub user_id: i32,
}

pub async fn add_member(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(id): Path<i32>,
    Json(payload): Json<AddMemberPayload>,
) -> AppResult<()> {
    ensure_owner(&pool, id, user_id).await?;
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id) VALUES ($1,$2) ON CONFLICT DO NOTHING"
    )
    .bind(id)
    .bind(payload.user_id)
    .execute(&pool)
    .await
    .map_err(|e| { tracing::error!(?e, "DB error adding member"); AppError::Db(e) })?;
    Ok(())
}

#[derive(serde::Serialize, sqlx::FromRow)]
pub struct OrganizationInvitation {
    pub id: Uuid,
    pub organization_id: i32,
    pub email: String,
    pub status: String,
    pub invited_at: DateTime<Utc>,
    pub accepted_at: Option<DateTime<Utc>>,
    pub expires_at: DateTime<Utc>,
    pub token: Uuid,
}

#[derive(serde::Deserialize)]
pub struct CreateInvitationRequest {
    pub email: String,
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
}

pub async fn list_invitations(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(id): Path<i32>,
) -> AppResult<Json<Vec<OrganizationInvitation>>> {
    ensure_owner(&pool, id, user_id).await?;
    let invites = sqlx::query_as::<_, OrganizationInvitation>(
        "SELECT id, organization_id, email, status, invited_at, accepted_at, expires_at, token \
         FROM organization_invitations WHERE organization_id = $1 ORDER BY invited_at DESC",
    )
    .bind(id)
    .fetch_all(&pool)
    .await
    .map_err(|e| {
        tracing::error!(?e, "DB error listing invitations");
        AppError::Db(e)
    })?;
    Ok(Json(invites))
}

pub async fn create_invitation(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(id): Path<i32>,
    Json(payload): Json<CreateInvitationRequest>,
) -> AppResult<Json<OrganizationInvitation>> {
    ensure_owner(&pool, id, user_id).await?;
    if payload.email.trim().is_empty() || !payload.email.contains('@') {
        return Err(AppError::BadRequest("Valid invite email required".into()));
    }
    let invitation_id = Uuid::new_v4();
    let token = Uuid::new_v4();
    let result = sqlx::query_as::<_, OrganizationInvitation>(
        "INSERT INTO organization_invitations (id, organization_id, invited_by, email, token, status, expires_at) \
         VALUES ($1, $2, $3, $4, $5, 'pending', COALESCE($6, NOW() + INTERVAL '14 days')) \
         RETURNING id, organization_id, email, status, invited_at, accepted_at, expires_at, token",
    )
    .bind(invitation_id)
    .bind(id)
    .bind(user_id)
    .bind(payload.email.trim())
    .bind(token)
    .bind(payload.expires_at)
    .fetch_one(&pool)
    .await;

    match result {
        Ok(record) => Ok(Json(record)),
        Err(sqlx::Error::Database(db_err)) => {
            if db_err.code().as_deref() == Some("23505") {
                Err(AppError::Conflict(
                    "Pending invite already exists for this email".into(),
                ))
            } else {
                tracing::error!(error = %db_err, "DB error creating invitation");
                Err(AppError::Db(sqlx::Error::Database(db_err)))
            }
        }
        Err(e) => {
            tracing::error!(?e, "DB error creating invitation");
            Err(AppError::Db(e))
        }
    }
}

pub async fn accept_invitation(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(token): Path<Uuid>,
) -> AppResult<Json<OrganizationInvitation>> {
    let mut tx = pool.begin().await.map_err(|e| AppError::Db(e))?;
    let invite = sqlx::query_as::<_, OrganizationInvitation>(
        "SELECT id, organization_id, email, status, invited_at, accepted_at, expires_at, token \
         FROM organization_invitations WHERE token = $1",
    )
    .bind(token)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!(?e, "DB error fetching invitation");
        AppError::Db(e)
    })?;

    let mut invite = invite.ok_or(AppError::NotFound)?;
    if invite.status != "pending" {
        return Err(AppError::Conflict("Invitation already processed".into()));
    }
    if invite.expires_at < Utc::now() {
        return Err(AppError::BadRequest("Invitation has expired".into()));
    }

    let user_email: Option<String> = sqlx::query_scalar("SELECT email FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| {
            tracing::error!(?e, "DB error loading user email");
            AppError::Db(e)
        })?;

    let Some(email) = user_email else {
        return Err(AppError::Unauthorized);
    };
    if email.trim().to_lowercase() != invite.email.trim().to_lowercase() {
        return Err(AppError::Forbidden);
    }

    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role) VALUES ($1, $2, 'member') \
         ON CONFLICT (organization_id, user_id) DO NOTHING",
    )
    .bind(invite.organization_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!(?e, "DB error adding invited member");
        AppError::Db(e)
    })?;

    invite = sqlx::query_as::<_, OrganizationInvitation>(
        "UPDATE organization_invitations SET status = 'accepted', accepted_at = NOW() \
         WHERE id = $1 RETURNING id, organization_id, email, status, invited_at, accepted_at, expires_at, token",
    )
    .bind(invite.id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!(?e, "DB error updating invitation status");
        AppError::Db(e)
    })?;

    tx.commit().await.map_err(|e| {
        tracing::error!(?e, "DB commit failed for invitation acceptance");
        AppError::Db(e)
    })?;

    Ok(Json(invite))
}

async fn ensure_owner(pool: &PgPool, organization_id: i32, user_id: i32) -> AppResult<()> {
    let rec = sqlx::query(
        "SELECT role FROM organization_members WHERE organization_id=$1 AND user_id=$2",
    )
    .bind(organization_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| {
        tracing::error!(?e, "DB error verifying organization owner");
        AppError::Db(e)
    })?;
    let Some(row) = rec else {
        return Err(AppError::Forbidden);
    };
    let role: String = row.get("role");
    if role != "owner" {
        return Err(AppError::Forbidden);
    }
    Ok(())
}
