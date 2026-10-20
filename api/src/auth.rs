//! Contraseñas (argon2id), JWT (HS256) en cookie httpOnly y extractor de usuario.

use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use std::sync::LazyLock;

use argon2::Argon2;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
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
    iat: i64,
    exp: i64,
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

    pub fn issue(&self, user_id: Uuid) -> Result<String, ApiError> {
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        let claims = Claims {
            sub: user_id,
            iat: now,
            exp: now + SESSION_DAYS * 24 * 3600,
        };
        jsonwebtoken::encode(&Header::new(Algorithm::HS256), &claims, &self.encoding)
            .map_err(ApiError::internal)
    }

    pub fn verify(&self, token: &str) -> Option<Uuid> {
        let validation = Validation::new(Algorithm::HS256);
        jsonwebtoken::decode::<Claims>(token, &self.decoding, &validation)
            .ok()
            .map(|d| d.claims.sub)
    }
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

/// Usuario autenticado por la cookie de sesión; si no hay o no es válida → 401.
#[derive(Debug, Clone, Copy)]
pub struct AuthUser {
    pub id: Uuid,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let jar = CookieJar::from_headers(&parts.headers);
        let token = jar.get(COOKIE_NAME).ok_or_else(ApiError::unauthorized)?;
        let id = state
            .jwt
            .verify(token.value())
            .ok_or_else(ApiError::unauthorized)?;
        Ok(AuthUser { id })
    }
}
