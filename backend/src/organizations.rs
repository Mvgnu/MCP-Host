use crate::error::{AppError, AppResult};
use crate::extractor::AuthUser;
use axum::extract::Path;
use axum::{
    routing::{get, post},
    Extension, Json, Router,
};
use sqlx::{PgPool, Row};

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
    // verify requester is owner
    let rec = sqlx::query(
        "SELECT role FROM organization_members WHERE organization_id=$1 AND user_id=$2",
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(&pool)
    .await
    .map_err(|e| {
        tracing::error!(?e, "DB error");
        AppError::Db(e)
    })?;
    let Some(row) = rec else {
        return Err(AppError::Forbidden);
    };
    let role: String = row.get("role");
    if role != "owner" {
        return Err(AppError::Forbidden);
    }
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
