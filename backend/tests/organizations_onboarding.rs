use axum::{extract::Path, Extension, Json};
use backend::extractor::AuthUser;
use backend::organizations::{
    accept_invitation, create_invitation, list_invitations, CreateInvitationRequest,
};
use sqlx::PgPool;
use uuid::Uuid;

// key: organizations-tests -> self-service-invitations
#[sqlx::test]
#[ignore = "requires DATABASE_URL with Postgres server"]
async fn owner_invites_and_member_accepts(pool: PgPool) {
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    let owner_id: i32 =
        sqlx::query_scalar("INSERT INTO users (email, password_hash) VALUES ($1, $2) RETURNING id")
            .bind("owner@example.com")
            .bind("hash")
            .fetch_one(&pool)
            .await
            .unwrap();

    let invitee_id: i32 =
        sqlx::query_scalar("INSERT INTO users (email, password_hash) VALUES ($1, $2) RETURNING id")
            .bind("invitee@example.com")
            .bind("hash")
            .fetch_one(&pool)
            .await
            .unwrap();

    let organization_id: i32 = sqlx::query_scalar(
        "INSERT INTO organizations (name, owner_id) VALUES ($1, $2) RETURNING id",
    )
    .bind("Acme Corp")
    .bind(owner_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role) VALUES ($1, $2, 'owner')",
    )
    .bind(organization_id)
    .bind(owner_id)
    .execute(&pool)
    .await
    .unwrap();

    let owner = AuthUser {
        user_id: owner_id,
        role: "owner".into(),
    };
    let Json(invitation) = create_invitation(
        Extension(pool.clone()),
        owner,
        Path(organization_id),
        Json(CreateInvitationRequest {
            email: "invitee@example.com".into(),
            expires_at: None,
        }),
    )
    .await
    .expect("owner can create invitation");

    assert_eq!(invitation.organization_id, organization_id);
    assert_eq!(invitation.email, "invitee@example.com");
    assert_eq!(invitation.status, "pending");

    let Json(invites) = list_invitations(
        Extension(pool.clone()),
        AuthUser {
            user_id: owner_id,
            role: "owner".into(),
        },
        Path(organization_id),
    )
    .await
    .expect("owner can list invitations");
    assert_eq!(invites.len(), 1);
    assert_eq!(invites[0].token, invitation.token);

    let Json(accepted) = accept_invitation(
        Extension(pool.clone()),
        AuthUser {
            user_id: invitee_id,
            role: "user".into(),
        },
        Path::<Uuid>(invitation.token),
    )
    .await
    .expect("invitee can accept");
    assert_eq!(accepted.status, "accepted");
    assert!(accepted.accepted_at.is_some());

    let membership: Option<String> = sqlx::query_scalar(
        "SELECT role FROM organization_members WHERE organization_id = $1 AND user_id = $2",
    )
    .bind(organization_id)
    .bind(invitee_id)
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(membership.as_deref(), Some("member"));
}

#[sqlx::test]
#[ignore = "requires DATABASE_URL with Postgres server"]
async fn non_owner_cannot_invite(pool: PgPool) {
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    let owner_id: i32 =
        sqlx::query_scalar("INSERT INTO users (email, password_hash) VALUES ($1, $2) RETURNING id")
            .bind("owner@example.com")
            .bind("hash")
            .fetch_one(&pool)
            .await
            .unwrap();

    let member_id: i32 =
        sqlx::query_scalar("INSERT INTO users (email, password_hash) VALUES ($1, $2) RETURNING id")
            .bind("member@example.com")
            .bind("hash")
            .fetch_one(&pool)
            .await
            .unwrap();

    let organization_id: i32 = sqlx::query_scalar(
        "INSERT INTO organizations (name, owner_id) VALUES ($1, $2) RETURNING id",
    )
    .bind("Acme Corp")
    .bind(owner_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role) VALUES ($1, $2, 'owner')",
    )
    .bind(organization_id)
    .bind(owner_id)
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role) VALUES ($1, $2, 'member')",
    )
    .bind(organization_id)
    .bind(member_id)
    .execute(&pool)
    .await
    .unwrap();

    let result = create_invitation(
        Extension(pool.clone()),
        AuthUser {
            user_id: member_id,
            role: "user".into(),
        },
        Path(organization_id),
        Json(CreateInvitationRequest {
            email: "friend@example.com".into(),
            expires_at: None,
        }),
    )
    .await;

    assert!(result.is_err());
}
