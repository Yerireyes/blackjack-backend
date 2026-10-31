//! Moderación: los usuarios reportan a otros (por nombre visible) y los administradores
//! (`ADMIN_EMAILS`) revisan los reportes, fuerzan un cambio de nombre o suspenden cuentas.
//! Una cuenta suspendida pierde todas sus sesiones, no puede volver a entrar y desaparece
//! de los rankings.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;

use crate::auth::{revoke_all_sessions, AdminUser, AuthUser};
use crate::error::{ApiError, ApiResult};
use crate::extract::{ApiJson, ApiPath};
use crate::names;
use crate::routes::auth::unique_violation;
use crate::state::AppState;

const REASONS: [&str; 4] = ["name", "behavior", "cheating", "other"];
const MAX_DETAILS: usize = 300;

/// `POST /reports`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportRequest {
    pub display_name: String,
    /// `name`, `behavior`, `cheating` u `other`.
    pub reason: String,
    pub details: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserRef {
    pub id: Uuid,
    pub display_name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportDto {
    pub id: Uuid,
    pub reporter: UserRef,
    pub target: UserRef,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(
        with = "time::serde::rfc3339::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub resolved_at: Option<OffsetDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminUserDto {
    pub id: Uuid,
    pub display_name: String,
    pub email: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(
        with = "time::serde::rfc3339::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub banned_at: Option<OffsetDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ban_reason: Option<String>,
    pub open_reports: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenamedDto {
    /// El nombre provisional; el usuario tiene que elegir otro.
    pub display_name: String,
}

#[derive(Deserialize)]
pub struct BanRequest {
    pub reason: String,
}

#[derive(Deserialize)]
pub struct ReportsQuery {
    status: Option<String>,
}

#[derive(Deserialize)]
pub struct UsersQuery {
    name: String,
}

fn user_not_found() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "user_not_found",
        "no hay nadie con ese nombre",
    )
}

fn name_key(raw: &str) -> String {
    raw.trim().nfc().collect::<String>().to_lowercase()
}

/// `POST /reports`: reporta a alguien por su nombre visible. Un solo reporte abierto de
/// cada persona sobre otra.
pub async fn report(
    State(state): State<AppState>,
    user: AuthUser,
    ApiJson(req): ApiJson<ReportRequest>,
) -> ApiResult<StatusCode> {
    if !REASONS.contains(&req.reason.as_str()) {
        return Err(ApiError::bad_request(
            "reason debe ser name, behavior, cheating u other",
        ));
    }
    let details = req
        .details
        .map(|d| d.trim().to_string())
        .filter(|d| !d.is_empty());
    if details
        .as_ref()
        .is_some_and(|d| d.chars().count() > MAX_DETAILS)
    {
        return Err(ApiError::bad_request(format!(
            "el detalle tiene como mucho {MAX_DETAILS} caracteres"
        )));
    }
    let target = sqlx::query_scalar!(
        "SELECT id FROM users WHERE display_name_key = $1",
        name_key(&req.display_name)
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(user_not_found)?;
    if target == user.id {
        return Err(ApiError::bad_request("no puedes reportarte a ti mismo"));
    }
    sqlx::query!(
        "INSERT INTO reports (id, reporter_id, target_id, reason, details)
         VALUES ($1, $2, $3, $4, $5)",
        Uuid::new_v4(),
        user.id,
        target,
        req.reason,
        details
    )
    .execute(&state.pool)
    .await
    .map_err(|e| match unique_violation(&e) {
        Some("reports_one_open") => ApiError::new(
            StatusCode::CONFLICT,
            "report_exists",
            "ya reportaste a esa persona; un administrador lo va a revisar",
        ),
        _ => ApiError::from(e),
    })?;
    Ok(StatusCode::ACCEPTED)
}

/// `GET /admin/reports?status=open|resolved`: los últimos 200, del más nuevo al más viejo.
pub async fn reports(
    State(state): State<AppState>,
    _admin: AdminUser,
    Query(q): Query<ReportsQuery>,
) -> ApiResult<Json<Vec<ReportDto>>> {
    let open = match q.status.as_deref() {
        None | Some("open") => true,
        Some("resolved") => false,
        Some(_) => return Err(ApiError::bad_request("status debe ser open o resolved")),
    };
    let rows = sqlx::query!(
        r#"SELECT r.id, r.reason, r.details, r.created_at, r.resolved_at, r.action,
                  r.reporter_id, a.display_name AS reporter_name,
                  r.target_id, b.display_name AS target_name
           FROM reports r
           JOIN users a ON a.id = r.reporter_id
           JOIN users b ON b.id = r.target_id
           WHERE (r.resolved_at IS NULL) = $1
           ORDER BY r.created_at DESC
           LIMIT 200"#,
        open
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| ReportDto {
                id: r.id,
                reporter: UserRef {
                    id: r.reporter_id,
                    display_name: r.reporter_name,
                },
                target: UserRef {
                    id: r.target_id,
                    display_name: r.target_name,
                },
                reason: r.reason,
                details: r.details,
                created_at: r.created_at,
                resolved_at: r.resolved_at,
                action: r.action,
            })
            .collect(),
    ))
}

/// `GET /admin/users?name=`: busca por el comienzo del nombre visible (hasta 20).
pub async fn users(
    State(state): State<AppState>,
    _admin: AdminUser,
    Query(q): Query<UsersQuery>,
) -> ApiResult<Json<Vec<AdminUserDto>>> {
    let key = name_key(&q.name);
    if key.is_empty() {
        return Err(ApiError::bad_request("name no puede estar vacío"));
    }
    // El prefijo va escapado: `%` y `_` del nombre se buscan tal cual.
    let pattern = format!(
        "{}%",
        key.replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    );
    let rows = sqlx::query_as!(
        AdminUserDto,
        r#"SELECT u.id, u.display_name, u.email, u.created_at, u.banned_at, u.ban_reason,
                  (SELECT count(*) FROM reports r
                   WHERE r.target_id = u.id AND r.resolved_at IS NULL) AS "open_reports!"
           FROM users u
           WHERE u.display_name_key LIKE $1
           ORDER BY u.display_name_key
           LIMIT 20"#,
        pattern
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(rows))
}

/// Cierra los reportes abiertos sobre `target` con la acción tomada.
async fn resolve_about(state: &AppState, admin: Uuid, target: Uuid, action: &str) -> ApiResult<()> {
    sqlx::query!(
        "UPDATE reports SET resolved_at = now(), resolved_by = $2, action = $3
         WHERE target_id = $1 AND resolved_at IS NULL",
        target,
        admin,
        action
    )
    .execute(&state.pool)
    .await?;
    Ok(())
}

/// `POST /admin/reports/{id}/dismiss`: cierra un reporte sin hacer nada.
pub async fn dismiss(
    State(state): State<AppState>,
    admin: AdminUser,
    ApiPath(id): ApiPath<Uuid>,
) -> ApiResult<StatusCode> {
    let done = sqlx::query!(
        "UPDATE reports SET resolved_at = now(), resolved_by = $2, action = 'dismiss'
         WHERE id = $1 AND resolved_at IS NULL",
        id,
        admin.id
    )
    .execute(&state.pool)
    .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::not_found("reporte no encontrado o ya resuelto"));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn target_email(state: &AppState, id: Uuid) -> ApiResult<String> {
    sqlx::query_scalar!("SELECT email FROM users WHERE id = $1", id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| ApiError::not_found("usuario no encontrado"))
}

/// `POST /admin/users/{id}/rename`: le pone un nombre provisional (`JugadorNNNN`) y le
/// pide elegir otro. Cierra los reportes abiertos sobre él.
pub async fn rename(
    State(state): State<AppState>,
    admin: AdminUser,
    ApiPath(id): ApiPath<Uuid>,
) -> ApiResult<Json<RenamedDto>> {
    target_email(&state, id).await?;
    for attempt in 0..10 {
        let name = names::provisional(attempt);
        let res = sqlx::query!(
            "UPDATE users SET display_name = $2, display_name_key = $3, display_name_set = false
             WHERE id = $1",
            id,
            name.name,
            name.key
        )
        .execute(&state.pool)
        .await;
        match res {
            Ok(_) => {
                resolve_about(&state, admin.id, id, "rename").await?;
                return Ok(Json(RenamedDto {
                    display_name: name.name,
                }));
            }
            Err(e) if unique_violation(&e) == Some("users_display_name_key") => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(ApiError::internal(
        "no se encontró un nombre provisional libre",
    ))
}

/// `POST /admin/users/{id}/ban` `{ reason }`: suspende la cuenta, cierra todas sus
/// sesiones y los reportes abiertos sobre ella.
pub async fn ban(
    State(state): State<AppState>,
    admin: AdminUser,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(req): ApiJson<BanRequest>,
) -> ApiResult<StatusCode> {
    let reason = req.reason.trim();
    if reason.is_empty() || reason.chars().count() > MAX_DETAILS {
        return Err(ApiError::bad_request(format!(
            "reason es obligatorio y tiene como mucho {MAX_DETAILS} caracteres"
        )));
    }
    let email = target_email(&state, id).await?;
    if id == admin.id || state.is_admin(&email) {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "cannot_ban_admin",
            "no se puede suspender a un administrador",
        ));
    }
    sqlx::query!(
        "UPDATE users SET banned_at = now(), ban_reason = $2 WHERE id = $1",
        id,
        reason
    )
    .execute(&state.pool)
    .await?;
    revoke_all_sessions(&state, id).await?;
    resolve_about(&state, admin.id, id, "ban").await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /admin/users/{id}/unban`.
pub async fn unban(
    State(state): State<AppState>,
    _admin: AdminUser,
    ApiPath(id): ApiPath<Uuid>,
) -> ApiResult<StatusCode> {
    let done = sqlx::query!(
        "UPDATE users SET banned_at = NULL, ban_reason = NULL WHERE id = $1",
        id
    )
    .execute(&state.pool)
    .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::not_found("usuario no encontrado"));
    }
    Ok(StatusCode::NO_CONTENT)
}
