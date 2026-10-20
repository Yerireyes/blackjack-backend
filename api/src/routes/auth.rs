//! `/auth/*`: registro, login y logout con JWT en cookie httpOnly.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use axum_extra::extract::cookie::CookieJar;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::auth::{hash_password, removal_cookie, session_cookie, verify_password, AuthUser};
use crate::error::{ApiError, ApiResult};
use crate::extract::ApiJson;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct Credentials {
    pub email: String,
    pub password: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserDto {
    pub id: Uuid,
    pub email: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

const MIN_PASSWORD: usize = 8;
const MAX_PASSWORD: usize = 128;

fn validate(c: &Credentials) -> ApiResult<String> {
    let email = c.email.trim().to_string();
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
    let len = c.password.chars().count();
    if !(MIN_PASSWORD..=MAX_PASSWORD).contains(&len) {
        return Err(ApiError::bad_request(format!(
            "la contraseña debe tener entre {MIN_PASSWORD} y {MAX_PASSWORD} caracteres"
        )));
    }
    Ok(email)
}

/// Crea el usuario y deja la sesión iniciada.
pub async fn register(
    State(state): State<AppState>,
    jar: CookieJar,
    ApiJson(creds): ApiJson<Credentials>,
) -> ApiResult<(StatusCode, CookieJar, Json<UserDto>)> {
    let email = validate(&creds)?;
    let hash = hash_password(creds.password).await?;
    let user = sqlx::query_as!(
        UserDto,
        r#"INSERT INTO users (id, email, password_hash) VALUES ($1, $2, $3)
           RETURNING id, email, created_at"#,
        Uuid::new_v4(),
        email,
        hash
    )
    .fetch_one(&state.pool)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db) if db.is_unique_violation() => ApiError::new(
            StatusCode::CONFLICT,
            "email_taken",
            "ya existe una cuenta con ese email",
        ),
        _ => ApiError::from(e),
    })?;
    let token = state.jwt.issue(user.id)?;
    let jar = jar.add(session_cookie(token, state.cookie_secure));
    Ok((StatusCode::CREATED, jar, Json(user)))
}

pub async fn login(
    State(state): State<AppState>,
    jar: CookieJar,
    ApiJson(creds): ApiJson<Credentials>,
) -> ApiResult<(CookieJar, Json<UserDto>)> {
    let row = sqlx::query!(
        r#"SELECT id, email, password_hash, created_at FROM users WHERE lower(email) = lower($1)"#,
        creds.email.trim()
    )
    .fetch_optional(&state.pool)
    .await?;
    let ok = verify_password(
        creds.password,
        row.as_ref().map(|r| r.password_hash.clone()),
    )
    .await?;
    let row = match row {
        Some(row) if ok => row,
        _ => {
            return Err(ApiError::new(
                StatusCode::UNAUTHORIZED,
                "invalid_credentials",
                "email o contraseña incorrectos",
            ))
        }
    };
    let token = state.jwt.issue(row.id)?;
    let jar = jar.add(session_cookie(token, state.cookie_secure));
    Ok((
        jar,
        Json(UserDto {
            id: row.id,
            email: row.email,
            created_at: row.created_at,
        }),
    ))
}

pub async fn logout(
    State(state): State<AppState>,
    _user: AuthUser,
    jar: CookieJar,
) -> (StatusCode, CookieJar) {
    (
        StatusCode::NO_CONTENT,
        jar.add(removal_cookie(state.cookie_secure)),
    )
}

/// Usuario de la sesión actual (para que el front sepa si hay sesión).
pub async fn me(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<UserDto>> {
    let user = sqlx::query_as!(
        UserDto,
        "SELECT id, email, created_at FROM users WHERE id = $1",
        user.id
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(ApiError::unauthorized)?;
    Ok(Json(user))
}
