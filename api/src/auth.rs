//! Contraseñas (argon2id), sesiones (JWT HS256 en cookie httpOnly, respaldado por
//! una fila revocable de `auth_sessions`) y extractor de usuario.

use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use std::sync::LazyLock;

use argon2::Argon2;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{header, HeaderMap};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiError;
use crate::state::AppState;

pub const COOKIE_NAME: &str = "bj_session";
/// Duración de la sesión.
pub const SESSION_DAYS: i64 = 7;

/// Hash de una contraseña cualquiera, para que un email inexistente tarde lo
/// mismo que uno existente al hacer login.
static DUMMY_HASH: LazyLock<String> = LazyLock::new(|| {
    Argon2::default()
        .hash_password(b"contrasena-ficticia")
        .expect("argon2 con parámetros por defecto")
        .to_string()
});

pub async fn hash_password(password: String) -> Result<String, ApiError> {
    tokio::task::spawn_blocking(move || {
        Argon2::default()
            .hash_password(password.as_bytes())
            .map(|h| h.to_string())
            .map_err(ApiError::internal)
    })
    .await
    .map_err(ApiError::internal)?
}

/// Verifica `password` contra `hash`; con `None` verifica contra un hash ficticio
/// y devuelve `false` (mismo tiempo de respuesta).
pub async fn verify_password(password: String, hash: Option<String>) -> Result<bool, ApiError> {
    tokio::task::spawn_blocking(move || {
        let known = hash.is_some();
        let hash = hash.unwrap_or_else(|| DUMMY_HASH.clone());
        let parsed = PasswordHash::new(&hash).map_err(ApiError::internal)?;
        let ok = Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok();
        Ok(ok && known)
    })
    .await
    .map_err(ApiError::internal)?
}

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    sub: Uuid,
    /// Fila de `auth_sessions`; sin ella el token no vale.
    sid: Uuid,
    iat: i64,
    exp: i64,
}

/// Lo que dice un JWT válido. Falta comprobar en la base que la sesión siga viva.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionToken {
    pub user_id: Uuid,
    pub session_id: Uuid,
}

#[derive(Clone)]
pub struct JwtKeys {
    encoding: EncodingKey,
    decoding: DecodingKey,
}

impl JwtKeys {
    pub fn new(secret: &[u8]) -> JwtKeys {
        JwtKeys {
            encoding: EncodingKey::from_secret(secret),
            decoding: DecodingKey::from_secret(secret),
        }
    }

    fn issue(&self, session: SessionToken, iat: i64, exp: i64) -> Result<String, ApiError> {
        let claims = Claims {
            sub: session.user_id,
            sid: session.session_id,
            iat,
            exp,
        };
        jsonwebtoken::encode(&Header::new(Algorithm::HS256), &claims, &self.encoding)
            .map_err(ApiError::internal)
    }

    pub fn verify(&self, token: &str) -> Option<SessionToken> {
        let validation = Validation::new(Algorithm::HS256);
        jsonwebtoken::decode::<Claims>(token, &self.decoding, &validation)
            .ok()
            .map(|d| SessionToken {
                user_id: d.claims.sub,
                session_id: d.claims.sid,
            })
    }
}

/// `User-Agent` guardado con la sesión (recortado), para listarlas algún día.
fn user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|ua| ua.chars().take(256).collect())
}

/// Crea una sesión para `user_id` y devuelve la cookie con su JWT.
pub async fn start_session(
    state: &AppState,
    user_id: Uuid,
    headers: &HeaderMap,
) -> Result<Cookie<'static>, ApiError> {
    let now = time::OffsetDateTime::now_utc();
    let expires = now + time::Duration::days(SESSION_DAYS);
    let session = SessionToken {
        user_id,
        session_id: Uuid::new_v4(),
    };
    sqlx::query!(
        "INSERT INTO auth_sessions (id, user_id, expires_at, user_agent) VALUES ($1, $2, $3, $4)",
        session.session_id,
        user_id,
        expires,
        user_agent(headers)
    )
    .execute(&state.pool)
    .await?;
    let token = state
        .jwt
        .issue(session, now.unix_timestamp(), expires.unix_timestamp())?;
    Ok(session_cookie(token, state.cookie_secure))
}

/// Revoca una sesión (logout).
pub async fn revoke_session(state: &AppState, session_id: Uuid) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE auth_sessions SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL",
        session_id
    )
    .execute(&state.pool)
    .await?;
    Ok(())
}

/// Revoca todas las sesiones del usuario (logout en todos los dispositivos).
pub async fn revoke_all_sessions(state: &AppState, user_id: Uuid) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE auth_sessions SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL",
        user_id
    )
    .execute(&state.pool)
    .await?;
    Ok(())
}

/// Borra las sesiones vencidas o revocadas hace más de un día. Devuelve cuántas.
pub async fn purge_sessions(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let done = sqlx::query!(
        "DELETE FROM auth_sessions
         WHERE expires_at < now() - interval '1 day' OR revoked_at < now() - interval '1 day'"
    )
    .execute(pool)
    .await?;
    Ok(done.rows_affected())
}

/// Cookie de sesión con el JWT.
pub fn session_cookie(token: String, secure: bool) -> Cookie<'static> {
    Cookie::build((COOKIE_NAME, token))
        .http_only(true)
        .secure(secure)
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(time::Duration::days(SESSION_DAYS))
        .build()
}

/// Cookie que borra la sesión.
pub fn removal_cookie(secure: bool) -> Cookie<'static> {
    Cookie::build((COOKIE_NAME, ""))
        .http_only(true)
        .secure(secure)
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(time::Duration::ZERO)
        .build()
}

/// Sesión de la cookie, si el JWT es válido y la sesión sigue viva en la base (no
/// revocada ni vencida). De paso actualiza `last_seen_at` si tiene más de 5 minutos.
pub async fn current_session(
    jar: &CookieJar,
    state: &AppState,
) -> Result<Option<SessionToken>, ApiError> {
    let Some(token) = jar
        .get(COOKIE_NAME)
        .and_then(|c| state.jwt.verify(c.value()))
    else {
        return Ok(None);
    };
    let alive = sqlx::query_scalar!(
        r#"WITH s AS (
               SELECT id, last_seen_at FROM auth_sessions
               WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL AND expires_at > now()
           ), seen AS (
               UPDATE auth_sessions SET last_seen_at = now()
               WHERE id IN (SELECT id FROM s WHERE last_seen_at < now() - interval '5 minutes')
           )
           SELECT EXISTS (SELECT 1 FROM s) AS "alive!""#,
        token.session_id,
        token.user_id
    )
    .fetch_one(&state.pool)
    .await?;
    Ok(alive.then_some(token))
}

/// Usuario autenticado por la cookie de sesión; si no hay, no es válida o la sesión
/// se revocó → 401.
#[derive(Debug, Clone, Copy)]
pub struct AuthUser {
    pub id: Uuid,
    pub session_id: Uuid,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let jar = CookieJar::from_headers(&parts.headers);
        let session = current_session(&jar, state)
            .await?
            .ok_or_else(ApiError::unauthorized)?;
        Ok(AuthUser {
            id: session.user_id,
            session_id: session.session_id,
        })
    }
}
