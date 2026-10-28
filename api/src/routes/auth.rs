//! `/auth/*` y `/me`: registro, login y logout con JWT en cookie httpOnly,
//! proveedores disponibles y nombre visible.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use axum_extra::extract::cookie::CookieJar;
use serde::{Deserialize, Serialize};
use sqlx::PgExecutor;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::auth::{
    hash_password, removal_cookie, revoke_all_sessions, revoke_session, start_session,
    verify_password, AuthUser,
};
use crate::error::{ApiError, ApiResult};
use crate::extract::ApiJson;
use crate::names::{self, DisplayName};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct Credentials {
    pub email: String,
    pub password: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Registration {
    pub email: String,
    pub password: String,
    pub display_name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileUpdate {
    pub display_name: String,
}

/// El usuario tal como lo ven register, login y `/auth/me`. Nunca incluye el hash.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeDto {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    /// El nombre es provisional (cuenta creada con Google o anterior a los nombres).
    pub needs_display_name: bool,
    pub has_password: bool,
    /// Proveedores externos vinculados (`"google"`).
    pub providers: Vec<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

#[derive(Serialize)]
pub struct ProvidersDto {
    pub google: bool,
}

const MIN_PASSWORD: usize = 8;
const MAX_PASSWORD: usize = 128;

fn validate(email: &str, password: &str) -> ApiResult<String> {
    let email = email.trim().to_string();
    let valid_email = email.len() <= 254
        && email.split_once('@').is_some_and(|(user, domain)| {
            !user.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
        })
        && !email.chars().any(char::is_whitespace);
    if !valid_email {
        return Err(ApiError::bad_request("email inválido"));
    }
    let len = password.chars().count();
    if !(MIN_PASSWORD..=MAX_PASSWORD).contains(&len) {
        return Err(ApiError::bad_request(format!(
            "la contraseña debe tener entre {MIN_PASSWORD} y {MAX_PASSWORD} caracteres"
        )));
    }
    Ok(email)
}

fn validate_name(raw: &str) -> ApiResult<DisplayName> {
    names::validate(raw)
        .map_err(|msg| ApiError::new(StatusCode::BAD_REQUEST, "display_name_invalid", msg))
}

pub fn display_name_taken() -> ApiError {
    ApiError::new(
        StatusCode::CONFLICT,
        "display_name_taken",
        "ese nombre ya está en uso",
    )
}

/// El índice único de `users` que violó un INSERT/UPDATE, si fue eso.
pub fn unique_violation(e: &sqlx::Error) -> Option<&str> {
    match e {
        sqlx::Error::Database(db) if db.is_unique_violation() => db.constraint(),
        _ => None,
    }
}

#[derive(Deserialize)]
pub struct PasswordConfirmation {
    pub password: String,
}

/// Comprueba la contraseña del usuario con sesión para una acción sensible.
/// Errores: 409 `no_password` (cuenta creada con Google) y 403 `invalid_password`
/// (no 401: para el front, un 401 con sesión significa "sesión cerrada").
pub async fn confirm_password(state: &AppState, user_id: Uuid, password: String) -> ApiResult<()> {
    let hash = sqlx::query_scalar!("SELECT password_hash FROM users WHERE id = $1", user_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(ApiError::unauthorized)?;
    let Some(hash) = hash else {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "no_password",
            "la cuenta no tiene contraseña",
        ));
    };
    if !verify_password(password, Some(hash)).await? {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "invalid_password",
            "contraseña incorrecta",
        ));
    }
    Ok(())
}

/// Carga el usuario con sus proveedores; `None` si ya no existe.
pub async fn load_me<'e>(db: impl PgExecutor<'e>, id: Uuid) -> ApiResult<Option<MeDto>> {
    let me = sqlx::query_as!(
        MeDto,
        r#"SELECT u.id, u.email, u.display_name,
                  NOT u.display_name_set AS "needs_display_name!",
                  u.password_hash IS NOT NULL AS "has_password!",
                  COALESCE(array_agg(i.provider ORDER BY i.provider)
                           FILTER (WHERE i.provider IS NOT NULL), '{}') AS "providers!",
                  u.created_at
           FROM users u LEFT JOIN identities i ON i.user_id = u.id
           WHERE u.id = $1
           GROUP BY u.id"#,
        id
    )
    .fetch_optional(db)
    .await?;
    Ok(me)
}

/// Crea el usuario y deja la sesión iniciada.
pub async fn register(
    State(state): State<AppState>,
    headers: HeaderMap,
    jar: CookieJar,
    ApiJson(reg): ApiJson<Registration>,
) -> ApiResult<(StatusCode, CookieJar, Json<MeDto>)> {
    let email = validate(&reg.email, &reg.password)?;
    let name = validate_name(&reg.display_name)?;
    let hash = hash_password(reg.password).await?;
    let user = sqlx::query!(
        r#"INSERT INTO users (id, email, password_hash, display_name, display_name_key, display_name_set)
           VALUES ($1, $2, $3, $4, $5, true)
           RETURNING id, email, display_name, created_at"#,
        Uuid::new_v4(),
        email,
        hash,
        name.name,
        name.key
    )
    .fetch_one(&state.pool)
    .await
    .map_err(|e| match unique_violation(&e) {
        Some("users_email_key") => ApiError::new(
            StatusCode::CONFLICT,
            "email_taken",
            "ya existe una cuenta con ese email",
        ),
        Some("users_display_name_key") => display_name_taken(),
        _ => ApiError::from(e),
    })?;
    let jar = jar.add(start_session(&state, user.id, &headers).await?);
    let me = MeDto {
        id: user.id,
        email: user.email,
        display_name: user.display_name,
        needs_display_name: false,
        has_password: true,
        providers: Vec::new(),
        created_at: user.created_at,
    };
    Ok((StatusCode::CREATED, jar, Json(me)))
}

/// Cuentas sin contraseña (creadas con Google) dan el mismo error genérico.
pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    jar: CookieJar,
    ApiJson(creds): ApiJson<Credentials>,
) -> ApiResult<(CookieJar, Json<MeDto>)> {
    let row = sqlx::query!(
        r#"SELECT id, password_hash FROM users WHERE lower(email) = lower($1)"#,
        creds.email.trim()
    )
    .fetch_optional(&state.pool)
    .await?;
    let ok = verify_password(
        creds.password,
        row.as_ref().and_then(|r| r.password_hash.clone()),
    )
    .await?;
    let id = match row {
        Some(row) if ok => row.id,
        _ => {
            return Err(ApiError::new(
                StatusCode::UNAUTHORIZED,
                "invalid_credentials",
                "email o contraseña incorrectos",
            ))
        }
    };
    let me = load_me(&state.pool, id)
        .await?
        .ok_or_else(ApiError::unauthorized)?;
    let jar = jar.add(start_session(&state, id, &headers).await?);
    Ok((jar, Json(me)))
}

/// Revoca la sesión actual: el token deja de valer aunque alguien lo haya copiado.
pub async fn logout(
    State(state): State<AppState>,
    user: AuthUser,
    jar: CookieJar,
) -> ApiResult<(StatusCode, CookieJar)> {
    revoke_session(&state, user.session_id).await?;
    Ok((
        StatusCode::NO_CONTENT,
        jar.add(removal_cookie(state.cookie_secure)),
    ))
}

/// Revoca todas las sesiones del usuario, incluida la actual.
pub async fn logout_all(
    State(state): State<AppState>,
    user: AuthUser,
    jar: CookieJar,
) -> ApiResult<(StatusCode, CookieJar)> {
    revoke_all_sessions(&state, user.id).await?;
    Ok((
        StatusCode::NO_CONTENT,
        jar.add(removal_cookie(state.cookie_secure)),
    ))
}

/// Usuario de la sesión actual (para que el front sepa si hay sesión).
pub async fn me(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<MeDto>> {
    let me = load_me(&state.pool, user.id)
        .await?
        .ok_or_else(ApiError::unauthorized)?;
    Ok(Json(me))
}

/// Qué formas de login externas están habilitadas (el front oculta las demás).
pub async fn providers(State(state): State<AppState>) -> Json<ProvidersDto> {
    Json(ProvidersDto {
        google: state.google.is_some(),
    })
}

/// Cambia el nombre visible. También confirma el provisional.
pub async fn update_me(
    State(state): State<AppState>,
    user: AuthUser,
    ApiJson(update): ApiJson<ProfileUpdate>,
) -> ApiResult<Json<MeDto>> {
    let name = validate_name(&update.display_name)?;
    let updated = sqlx::query!(
        r#"UPDATE users SET display_name = $2, display_name_key = $3, display_name_set = true
           WHERE id = $1"#,
        user.id,
        name.name,
        name.key
    )
    .execute(&state.pool)
    .await
    .map_err(|e| match unique_violation(&e) {
        Some("users_display_name_key") => display_name_taken(),
        _ => ApiError::from(e),
    })?;
    if updated.rows_affected() == 0 {
        return Err(ApiError::unauthorized());
    }
    me(State(state), user).await
}
