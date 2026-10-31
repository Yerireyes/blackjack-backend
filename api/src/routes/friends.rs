//! Amigos: solicitudes por nombre visible, lista con quién está conectado y en qué sala
//! pública juega, e invitaciones a salas. Las invitaciones viven en la base pero la sala
//! en memoria: solo se muestran mientras la sala siga abierta y por 30 minutos.

use std::collections::HashSet;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use engine::game::{
    FriendRequest, FriendRequestView, FriendView, FriendsView, InviteRequest, RoomInvitation,
    RoomPlayer,
};
use sqlx::PgConnection;
use time::{Duration, OffsetDateTime};
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::extract::{ApiJson, ApiPath};
use crate::routes::casual::{lock_user, rfc3339};
use crate::routes::cosmetics::title;
use crate::state::AppState;

pub const MAX_FRIENDS: i64 = 200;
pub const MAX_OUTGOING: i64 = 50;
/// Uso reciente que cuenta como «conectado».
const ONLINE: Duration = Duration::minutes(10);
pub const INVITE_TTL: Duration = Duration::minutes(30);

fn conflict(code: &'static str, message: &str) -> ApiError {
    ApiError::new(StatusCode::CONFLICT, code, message)
}

fn player(id: Uuid, name: String, t: Option<String>) -> RoomPlayer {
    RoomPlayer {
        id: id.to_string(),
        display_name: name,
        title: title(t.as_deref()),
    }
}

async fn are_friends(db: &mut PgConnection, a: Uuid, b: Uuid) -> ApiResult<bool> {
    let yes = sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM friends WHERE user_id = $1 AND friend_id = $2) AS "yes!""#,
        a,
        b
    )
    .fetch_one(db)
    .await?;
    Ok(yes)
}

/// Los amigos del usuario (ids), para el ranking entre amigos.
pub async fn friend_ids(db: &mut PgConnection, user: Uuid) -> ApiResult<Vec<Uuid>> {
    let ids = sqlx::query_scalar!("SELECT friend_id FROM friends WHERE user_id = $1", user)
        .fetch_all(db)
        .await?;
    Ok(ids)
}

/// `GET /friends`.
pub async fn list(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<FriendsView>> {
    let mut db = state.pool.acquire().await?;
    let since_online = OffsetDateTime::now_utc() - ONLINE;
    let rows = sqlx::query!(
        r#"SELECT f.friend_id AS "friend_id!", f.since AS "since!",
                  u.display_name AS "display_name!",
                  c.title AS "title?",
                  EXISTS (SELECT 1 FROM auth_sessions s
                          WHERE s.user_id = f.friend_id AND s.revoked_at IS NULL
                            AND s.expires_at > now() AND s.last_seen_at > $2) AS "recent!"
           FROM friends f
           JOIN users u ON u.id = f.friend_id
           LEFT JOIN user_cosmetics c ON c.user_id = f.friend_id
           WHERE f.user_id = $1"#,
        user.id,
        since_online
    )
    .fetch_all(&mut *db)
    .await?;
    let mut friends: Vec<FriendView> = rows
        .into_iter()
        .map(|r| {
            let room = state.rooms.public_room_of(r.friend_id);
            FriendView {
                online: r.recent || room.is_some(),
                room,
                since: rfc3339(r.since),
                user: player(r.friend_id, r.display_name, r.title),
            }
        })
        .collect();
    friends.sort_by(|a, b| {
        b.online.cmp(&a.online).then_with(|| {
            a.user
                .display_name
                .to_lowercase()
                .cmp(&b.user.display_name.to_lowercase())
        })
    });

    let requests = sqlx::query!(
        r#"SELECT r.from_id, r.to_id, r.created_at, u.id AS other, u.display_name,
                  c.title AS "title?"
           FROM friend_requests r
           JOIN users u ON u.id = CASE WHEN r.from_id = $1 THEN r.to_id ELSE r.from_id END
           LEFT JOIN user_cosmetics c ON c.user_id = u.id
           WHERE r.from_id = $1 OR r.to_id = $1
           ORDER BY r.created_at DESC"#,
        user.id
    )
    .fetch_all(&mut *db)
    .await?;
    let (mut incoming, mut outgoing) = (Vec::new(), Vec::new());
    for r in requests {
        let view = FriendRequestView {
            user: player(r.other, r.display_name, r.title),
            at: rfc3339(r.created_at),
        };
        if r.to_id == user.id {
            incoming.push(view);
        } else {
            outgoing.push(view);
        }
    }

    let invites = sqlx::query!(
        r#"SELECT i.id, i.room_code, i.created_at, i.from_id, u.display_name,
                  c.title AS "title?"
           FROM room_invites i
           JOIN users u ON u.id = i.from_id
           LEFT JOIN user_cosmetics c ON c.user_id = i.from_id
           WHERE i.to_id = $1 AND i.created_at > $2
           ORDER BY i.created_at DESC"#,
        user.id,
        OffsetDateTime::now_utc() - INVITE_TTL
    )
    .fetch_all(&mut *db)
    .await?;
    let invitations = invites
        .into_iter()
        .filter_map(|r| {
            // Solo las de salas que siguen abiertas.
            let room = state.rooms.info(&r.room_code)?;
            Some(RoomInvitation {
                id: r.id.to_string(),
                from: player(r.from_id, r.display_name, r.title),
                room,
                at: rfc3339(r.created_at),
            })
        })
        .collect();

    Ok(Json(FriendsView {
        friends,
        incoming,
        outgoing,
        invitations,
    }))
}

/// `POST /friends/requests` `{ displayName }`: pide amistad. Si el otro ya la había
/// pedido, quedan como amigos.
pub async fn request(
    State(state): State<AppState>,
    user: AuthUser,
    ApiJson(req): ApiJson<FriendRequest>,
) -> ApiResult<StatusCode> {
    let key = req
        .display_name
        .trim()
        .nfc()
        .collect::<String>()
        .to_lowercase();
    let mut tx = state.pool.begin().await?;
    lock_user(&mut tx, user.id).await?;
    let other = sqlx::query_scalar!("SELECT id FROM users WHERE display_name_key = $1", key)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                "user_not_found",
                "no hay nadie con ese nombre",
            )
        })?;
    if other == user.id {
        return Err(ApiError::bad_request("no puedes agregarte a ti mismo"));
    }
    if are_friends(&mut tx, user.id, other).await? {
        return Err(conflict("already_friends", "ya son amigos"));
    }
    let reverse = sqlx::query!(
        "DELETE FROM friend_requests WHERE from_id = $1 AND to_id = $2",
        other,
        user.id
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if reverse > 0 {
        befriend(&mut tx, user.id, other).await?;
        tx.commit().await?;
        return Ok(StatusCode::CREATED);
    }
    let counts = sqlx::query!(
        r#"SELECT (SELECT count(*) FROM friends WHERE user_id = $1) AS "friends!",
                  (SELECT count(*) FROM friend_requests WHERE from_id = $1) AS "outgoing!""#,
        user.id
    )
    .fetch_one(&mut *tx)
    .await?;
    if counts.friends >= MAX_FRIENDS {
        return Err(conflict("too_many_friends", "llegaste al máximo de amigos"));
    }
    if counts.outgoing >= MAX_OUTGOING {
        return Err(conflict(
            "too_many_requests",
            "tienes demasiadas solicitudes pendientes",
        ));
    }
    let inserted = sqlx::query!(
        "INSERT INTO friend_requests (from_id, to_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        user.id,
        other
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if inserted == 0 {
        return Err(conflict("request_exists", "ya le mandaste una solicitud"));
    }
    tx.commit().await?;
    Ok(StatusCode::ACCEPTED)
}

async fn befriend(db: &mut PgConnection, a: Uuid, b: Uuid) -> ApiResult<()> {
    sqlx::query!(
        "INSERT INTO friends (user_id, friend_id) VALUES ($1, $2), ($2, $1)
         ON CONFLICT DO NOTHING",
        a,
        b
    )
    .execute(db)
    .await?;
    Ok(())
}

/// `POST /friends/requests/{userId}/accept`.
pub async fn accept(
    State(state): State<AppState>,
    user: AuthUser,
    ApiPath(other): ApiPath<Uuid>,
) -> ApiResult<StatusCode> {
    let mut tx = state.pool.begin().await?;
    lock_user(&mut tx, user.id).await?;
    let found = sqlx::query!(
        "DELETE FROM friend_requests WHERE from_id = $1 AND to_id = $2",
        other,
        user.id
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if found == 0 {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "request_not_found",
            "no hay una solicitud de esa persona",
        ));
    }
    befriend(&mut tx, user.id, other).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /friends/requests/{userId}/decline`: rechaza una recibida o cancela una enviada.
pub async fn decline(
    State(state): State<AppState>,
    user: AuthUser,
    ApiPath(other): ApiPath<Uuid>,
) -> ApiResult<StatusCode> {
    let found = sqlx::query!(
        "DELETE FROM friend_requests
         WHERE (from_id = $1 AND to_id = $2) OR (from_id = $2 AND to_id = $1)",
        other,
        user.id
    )
    .execute(&state.pool)
    .await?
    .rows_affected();
    if found == 0 {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "request_not_found",
            "no hay una solicitud con esa persona",
        ));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /friends/{userId}/remove`: deja de ser amigos (los dos lados).
pub async fn remove(
    State(state): State<AppState>,
    user: AuthUser,
    ApiPath(other): ApiPath<Uuid>,
) -> ApiResult<StatusCode> {
    let found = sqlx::query!(
        "DELETE FROM friends
         WHERE (user_id = $1 AND friend_id = $2) OR (user_id = $2 AND friend_id = $1)",
        user.id,
        other
    )
    .execute(&state.pool)
    .await?
    .rows_affected();
    if found == 0 {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "not_friends",
            "no son amigos",
        ));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /rooms/{code}/invite` `{ userId }`: invita a un amigo a la sala en la que está
/// el usuario (vale también para salas privadas).
pub async fn invite(
    State(state): State<AppState>,
    user: AuthUser,
    ApiPath(code): ApiPath<String>,
    ApiJson(req): ApiJson<InviteRequest>,
) -> ApiResult<StatusCode> {
    let to: Uuid = req
        .user_id
        .parse()
        .map_err(|_| ApiError::bad_request("userId debe ser un UUID"))?;
    let room = state.rooms.info(&code).ok_or_else(|| {
        ApiError::new(
            StatusCode::NOT_FOUND,
            "room_not_found",
            "esa sala no existe o ya cerró",
        )
    })?;
    if !state.rooms.is_member(&room.code, user.id) {
        return Err(conflict("not_in_room", "entra a la sala para invitar"));
    }
    let mut db = state.pool.acquire().await?;
    if !are_friends(&mut db, user.id, to).await? {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "not_friends",
            "solo puedes invitar a tus amigos",
        ));
    }
    // Volver a invitar renueva la invitación.
    sqlx::query!(
        "INSERT INTO room_invites (id, from_id, to_id, room_code) VALUES ($1, $2, $3, $4)
         ON CONFLICT (from_id, to_id, room_code) DO UPDATE SET created_at = now()",
        Uuid::new_v4(),
        user.id,
        to,
        room.code
    )
    .execute(&mut *db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /me/invitations/{id}/dismiss`.
pub async fn dismiss(
    State(state): State<AppState>,
    user: AuthUser,
    ApiPath(id): ApiPath<Uuid>,
) -> ApiResult<StatusCode> {
    sqlx::query!(
        "DELETE FROM room_invites WHERE id = $1 AND to_id = $2",
        id,
        user.id
    )
    .execute(&state.pool)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Borra las invitaciones vencidas (la tarea horaria).
pub async fn purge_invites(pool: &sqlx::PgPool) -> Result<u64, sqlx::Error> {
    let done = sqlx::query!(
        "DELETE FROM room_invites WHERE created_at < $1",
        OffsetDateTime::now_utc() - INVITE_TTL
    )
    .execute(pool)
    .await?;
    Ok(done.rows_affected())
}

/// Ids del usuario y sus amigos, para filtrar un ranking.
pub async fn circle(db: &mut PgConnection, user: Uuid) -> ApiResult<HashSet<Uuid>> {
    let mut ids: HashSet<Uuid> = friend_ids(db, user).await?.into_iter().collect();
    ids.insert(user);
    Ok(ids)
}
