//! Login con Google: OAuth 2.0 Authorization Code + PKCE.
//!
//! `start` guarda `state` y `code_verifier` en una cookie cifrada de 10 min y
//! redirige a Google. `callback` comprueba `state`, cambia el código por tokens
//! desde el servidor y resuelve la cuenta:
//!
//! - **Login:** por `identities(google, sub)`; si no existe y el email
//!   verificado no tiene cuenta, se crea una con nombre provisional. Si el email
//!   ya tiene cuenta, **no** se vincula sola (`account_exists`): el email de una
//!   cuenta con contraseña no está verificado y vincularla permitiría que quien
//!   la registró con un email ajeno comparta la cuenta del dueño real.
//! - **Vincular** (`start?link=1` con sesión): agrega Google a la cuenta actual.
//!   Antes hay que confirmar la contraseña con `POST /auth/google/link-intent`; sin
//!   esa prueba vigente la vuelta es `?link=error&code=reauth_required`.
//! - **Desvincular** (`POST /auth/google/unlink`, con la contraseña).
//!
//! El navegador vuelve a `FRONTEND_ORIGIN` con `?login=ok`, `?link=ok` o
//! `?login|link=error&code=...`.

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Redirect;
use axum::Json;
use axum_extra::extract::cookie::{Cookie, CookieJar, PrivateCookieJar, SameSite};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::auth::{current_session, start_session, AuthUser};
use crate::config::GoogleConfig;
use crate::error::{ApiError, ApiResult};
use crate::extract::ApiJson;
use crate::names;
use crate::routes::auth::{
    confirm_password, load_me, unique_violation, MeDto, PasswordConfirmation,
};
use crate::state::{AppState, Google};

/// Vida de la prueba de `link-intent`.
pub const LINK_INTENT_MINUTES: i64 = 5;
const OAUTH_COOKIE: &str = "bj_oauth";
/// La cookie solo viaja a `/auth/google/*`.
const OAUTH_PATH: &str = "/api/v1/auth/google";
const OAUTH_MINUTES: i64 = 10;
const PROVIDER: &str = "google";
const GOOGLE_ISSUERS: [&str; 2] = ["https://accounts.google.com", "accounts.google.com"];

/// Lo que `start` deja en la cookie cifrada para `callback`.
#[derive(Serialize, Deserialize)]
struct Pending {
    state: String,
    verifier: String,
    /// Usuario al que se vincula Google; `None` en un login.
    link: Option<Uuid>,
    /// Unix; la cookie también caduca, pero esto no depende del navegador.
    exp: i64,
}

#[derive(Deserialize)]
pub struct StartQuery {
    link: Option<String>,
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

/// Datos del `id_token` que se usan.
#[derive(Debug, Deserialize)]
pub struct IdClaims {
    iss: String,
    aud: String,
    exp: i64,
    pub sub: String,
    pub email: Option<String>,
    #[serde(default)]
    pub email_verified: bool,
}

#[derive(Clone, Copy)]
enum Mode {
    Login,
    Link,
}

impl Mode {
    fn param(self) -> &'static str {
        match self {
            Mode::Login => "login",
            Mode::Link => "link",
        }
    }
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// `code_challenge` S256 de PKCE.
fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn front(state: &AppState, mode: Mode, result: Result<(), &str>) -> Redirect {
    let query = match result {
        Ok(()) => format!("{}=ok", mode.param()),
        Err(code) => format!("{}=error&code={code}", mode.param()),
    };
    Redirect::to(&format!("{}/?{query}", state.frontend_origin))
}

fn oauth_cookie(value: String, secure: bool) -> Cookie<'static> {
    Cookie::build((OAUTH_COOKIE, value))
        .http_only(true)
        .secure(secure)
        .same_site(SameSite::Lax)
        .path(OAUTH_PATH)
        .max_age(time::Duration::minutes(OAUTH_MINUTES))
        .build()
}

/// Gasta la prueba de `link-intent` de la sesión: `true` si había una vigente.
async fn consume_link_intent(state: &AppState, session_id: Uuid) -> Result<bool, sqlx::Error> {
    let done = sqlx::query!(
        "UPDATE auth_sessions SET link_intent_until = NULL
         WHERE id = $1 AND link_intent_until > now()",
        session_id
    )
    .execute(&state.pool)
    .await?;
    Ok(done.rows_affected() == 1)
}

fn google_disabled() -> ApiError {
    ApiError::new(
        StatusCode::CONFLICT,
        "google_disabled",
        "el acceso con Google no está disponible",
    )
}

/// `POST /auth/google/link-intent`: confirma la contraseña antes de vincular. Deja en
/// la sesión una prueba de un solo uso que vence en [`LINK_INTENT_MINUTES`] y que
/// `start?link=1` exige. Así un token de sesión robado no basta para vincular otra
/// cuenta de Google y conservar el acceso.
pub async fn link_intent(
    State(state): State<AppState>,
    user: AuthUser,
    ApiJson(body): ApiJson<PasswordConfirmation>,
) -> ApiResult<StatusCode> {
    if state.google.is_none() {
        return Err(google_disabled());
    }
    confirm_password(&state, user.id, body.password).await?;
    let linked = sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM identities WHERE user_id = $1 AND provider = $2) AS "linked!""#,
        user.id,
        PROVIDER
    )
    .fetch_one(&state.pool)
    .await?;
    if linked {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "already_linked",
            "la cuenta ya tiene Google vinculado",
        ));
    }
    sqlx::query!(
        "UPDATE auth_sessions SET link_intent_until = now() + make_interval(mins => $2)
         WHERE id = $1",
        user.session_id,
        LINK_INTENT_MINUTES as i32
    )
    .execute(&state.pool)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /auth/google/unlink`: quita Google con la contraseña. Rechaza cuentas sin
/// contraseña (se quedarían sin forma de entrar). Revoca las demás sesiones del
/// usuario: si alguien había vinculado su Google con un token robado, también pierde
/// las sesiones que abrió con él. Hace falta la contraseña, así que no sirve para
/// echar al dueño.
pub async fn unlink(
    State(state): State<AppState>,
    user: AuthUser,
    ApiJson(body): ApiJson<PasswordConfirmation>,
) -> ApiResult<Json<MeDto>> {
    confirm_password(&state, user.id, body.password).await?;
    let mut tx = state.pool.begin().await?;
    let removed = sqlx::query!(
        "DELETE FROM identities WHERE user_id = $1 AND provider = $2",
        user.id,
        PROVIDER
    )
    .execute(&mut *tx)
    .await?;
    if removed.rows_affected() == 0 {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "not_linked",
            "la cuenta no tiene Google vinculado",
        ));
    }
    sqlx::query!(
        "UPDATE auth_sessions SET revoked_at = now()
         WHERE user_id = $1 AND id <> $2 AND revoked_at IS NULL",
        user.id,
        user.session_id
    )
    .execute(&mut *tx)
    .await?;
    let me = load_me(&mut *tx, user.id)
        .await?
        .ok_or_else(ApiError::unauthorized)?;
    tx.commit().await?;
    Ok(Json(me))
}

pub async fn start(
    State(state): State<AppState>,
    Query(query): Query<StartQuery>,
    session: CookieJar,
    jar: PrivateCookieJar,
) -> (PrivateCookieJar, Redirect) {
    let mode = match query.link.as_deref() {
        Some("1" | "true") => Mode::Link,
        _ => Mode::Login,
    };
    let Some(google) = state.google.clone() else {
        return (jar, front(&state, mode, Err("google_disabled")));
    };
    let link = match mode {
        Mode::Login => None,
        Mode::Link => {
            let s = match current_session(&session, &state).await {
                Ok(Some(s)) => s,
                Ok(None) => return (jar, front(&state, mode, Err("unauthorized"))),
                Err(_) => return (jar, front(&state, mode, Err("internal"))),
            };
            match consume_link_intent(&state, s.session_id).await {
                Ok(true) => Some(s.user_id),
                Ok(false) => return (jar, front(&state, mode, Err("reauth_required"))),
                Err(_) => return (jar, front(&state, mode, Err("internal"))),
            }
        }
    };
    let pending = Pending {
        state: random_token(),
        verifier: random_token(),
        link,
        exp: OffsetDateTime::now_utc().unix_timestamp() + OAUTH_MINUTES * 60,
    };
    let cfg = &google.config;
    let url = reqwest::Url::parse_with_params(
        &cfg.auth_url,
        [
            ("client_id", cfg.client_id.as_str()),
            ("redirect_uri", cfg.redirect_uri.as_str()),
            ("response_type", "code"),
            ("scope", "openid email profile"),
            ("state", pending.state.as_str()),
            ("code_challenge", challenge(&pending.verifier).as_str()),
            ("code_challenge_method", "S256"),
            ("prompt", "select_account"),
        ],
    );
    let url = match url {
        Ok(url) => url,
        Err(e) => {
            ApiError::internal(format!("GOOGLE auth_url inválida: {e}"));
            return (jar, front(&state, mode, Err("google_error")));
        }
    };
    let value = serde_json::to_string(&pending).expect("Pending se serializa");
    let jar = jar.add(oauth_cookie(value, state.cookie_secure));
    (jar, Redirect::to(url.as_str()))
}

pub async fn callback(
    State(state): State<AppState>,
    Query(query): Query<CallbackQuery>,
    headers: HeaderMap,
    session: CookieJar,
    jar: PrivateCookieJar,
) -> (PrivateCookieJar, CookieJar, Redirect) {
    // La cookie se usa una sola vez, salga bien o mal.
    let pending = jar
        .get(OAUTH_COOKIE)
        .and_then(|c| serde_json::from_str::<Pending>(c.value()).ok());
    let jar = jar.remove(Cookie::build(OAUTH_COOKIE).path(OAUTH_PATH));
    let mode = match pending.as_ref().and_then(|p| p.link) {
        Some(_) => Mode::Link,
        None => Mode::Login,
    };
    match finish(&state, query, &session, pending).await {
        Ok(Some(user_id)) => match start_session(&state, user_id, &headers).await {
            Ok(cookie) => (jar, session.add(cookie), front(&state, mode, Ok(()))),
            Err(_) => (jar, session, front(&state, mode, Err("internal"))),
        },
        Ok(None) => (jar, session, front(&state, mode, Ok(()))),
        Err(code) => (jar, session, front(&state, mode, Err(code))),
    }
}

/// Resuelve el callback. `Ok(Some(id))` inicia sesión como `id`; `Ok(None)` es
/// una vinculación correcta (la sesión ya existe); `Err(code)` va al front.
async fn finish(
    state: &AppState,
    query: CallbackQuery,
    session: &CookieJar,
    pending: Option<Pending>,
) -> Result<Option<Uuid>, &'static str> {
    let google = state.google.clone().ok_or("google_disabled")?;
    let pending = pending.ok_or("invalid_state")?;
    if pending.exp < OffsetDateTime::now_utc().unix_timestamp() {
        return Err("invalid_state");
    }
    if query.error.is_some() {
        return Err("cancelled");
    }
    if query.state.as_deref() != Some(pending.state.as_str()) {
        return Err("invalid_state");
    }
    let code = query.code.ok_or("invalid_state")?;
    let claims = exchange(&google, &code, &pending.verifier)
        .await
        .map_err(|e| {
            tracing::warn!("Google: {e}");
            "google_error"
        })?;
    let result = match pending.link {
        Some(user_id) => {
            // Quien vincula tiene que seguir siendo el usuario de una sesión viva.
            let current = current_session(session, state)
                .await
                .map_err(|_| "internal")?;
            if current.map(|s| s.user_id) != Some(user_id) {
                return Err("unauthorized");
            }
            link(state, user_id, &claims).await.map(|()| None)
        }
        None => login(state, &claims).await.map(Some),
    };
    result.map_err(|e| match e {
        Outcome::Code(code) => code,
        Outcome::Internal => "internal",
    })
}

enum Outcome {
    /// Error esperado, con su código para el front.
    Code(&'static str),
    /// Error interno; `ApiError` ya lo registró en el log al crearse.
    Internal,
}

impl From<sqlx::Error> for Outcome {
    fn from(e: sqlx::Error) -> Outcome {
        ApiError::from(e).into()
    }
}

impl From<ApiError> for Outcome {
    fn from(_: ApiError) -> Outcome {
        Outcome::Internal
    }
}

async fn identity_owner(state: &AppState, sub: &str) -> Result<Option<Uuid>, Outcome> {
    let owner = sqlx::query_scalar!(
        "SELECT user_id FROM identities WHERE provider = $1 AND subject = $2",
        PROVIDER,
        sub
    )
    .fetch_optional(&state.pool)
    .await?;
    Ok(owner)
}

async fn link(state: &AppState, user_id: Uuid, claims: &IdClaims) -> Result<(), Outcome> {
    match identity_owner(state, &claims.sub).await? {
        Some(owner) if owner == user_id => return Ok(()),
        Some(_) => return Err(Outcome::Code("identity_in_use")),
        None => {}
    }
    sqlx::query!(
        "INSERT INTO identities (id, user_id, provider, subject, email) VALUES ($1, $2, $3, $4, $5)",
        Uuid::new_v4(),
        user_id,
        PROVIDER,
        claims.sub,
        claims.email
    )
    .execute(&state.pool)
    .await
    .map_err(|e| match unique_violation(&e) {
        Some("identities_user_id_provider_key") => Outcome::Code("already_linked"),
        Some("identities_provider_subject_key") => Outcome::Code("identity_in_use"),
        _ => Outcome::from(e),
    })?;
    Ok(())
}

async fn login(state: &AppState, claims: &IdClaims) -> Result<Uuid, Outcome> {
    if let Some(owner) = identity_owner(state, &claims.sub).await? {
        return Ok(owner);
    }
    let email = match &claims.email {
        Some(email) if claims.email_verified => email,
        _ => return Err(Outcome::Code("email_unverified")),
    };
    let taken = sqlx::query_scalar!(
        "SELECT EXISTS (SELECT 1 FROM users WHERE lower(email) = lower($1)) AS \"taken!\"",
        email
    )
    .fetch_one(&state.pool)
    .await?;
    if taken {
        return Err(Outcome::Code("account_exists"));
    }
    // Nombre provisional: se reintenta si choca con uno existente.
    for attempt in 0..10 {
        match create_user(state, email, &claims.sub, names::provisional(attempt)).await {
            Ok(id) => return Ok(id),
            Err(e) => match unique_violation(&e) {
                Some("users_display_name_key") => continue,
                Some("users_email_key") => return Err(Outcome::Code("account_exists")),
                // Otro callback simultáneo creó la misma identidad.
                Some("identities_provider_subject_key") => {
                    return identity_owner(state, &claims.sub)
                        .await?
                        .ok_or(Outcome::Code("internal"));
                }
                _ => return Err(e.into()),
            },
        }
    }
    Err(ApiError::internal("no se encontró un nombre provisional libre").into())
}

async fn create_user(
    state: &AppState,
    email: &str,
    sub: &str,
    name: names::DisplayName,
) -> Result<Uuid, sqlx::Error> {
    let mut tx = state.pool.begin().await?;
    let id = Uuid::new_v4();
    sqlx::query!(
        r#"INSERT INTO users (id, email, password_hash, display_name, display_name_key, display_name_set)
           VALUES ($1, $2, NULL, $3, $4, false)"#,
        id,
        email,
        name.name,
        name.key
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "INSERT INTO identities (id, user_id, provider, subject, email) VALUES ($1, $2, $3, $4, $5)",
        Uuid::new_v4(),
        id,
        PROVIDER,
        sub,
        email
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(id)
}

#[derive(Deserialize)]
struct TokenResponse {
    id_token: String,
}

/// Cambia el código por tokens y devuelve los datos validados del `id_token`.
async fn exchange(google: &Google, code: &str, verifier: &str) -> Result<IdClaims, String> {
    let cfg = &google.config;
    let res = google
        .http
        .post(&cfg.token_url)
        .form(&[
            ("code", code),
            ("client_id", cfg.client_id.as_str()),
            ("client_secret", cfg.client_secret.as_str()),
            ("redirect_uri", cfg.redirect_uri.as_str()),
            ("grant_type", "authorization_code"),
            ("code_verifier", verifier),
        ])
        .send()
        .await
        .map_err(|e| format!("intercambio del código: {e}"))?;
    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        return Err(format!("intercambio del código: {status} {body}"));
    }
    let token: TokenResponse = res
        .json()
        .await
        .map_err(|e| format!("respuesta de tokens: {e}"))?;
    let now = OffsetDateTime::now_utc().unix_timestamp();
    parse_id_token(&token.id_token, cfg, now)
}

/// Lee y valida los datos del `id_token`. No se verifica la firma: el token llega
/// directo del endpoint de tokens de Google por TLS, y OpenID Connect Core
/// (3.1.3.7) permite confiar en él en ese caso. Sí se comprueban `iss`, `aud` y `exp`.
pub fn parse_id_token(token: &str, cfg: &GoogleConfig, now: i64) -> Result<IdClaims, String> {
    let payload = token.split('.').nth(1).ok_or("id_token mal formado")?;
    let bytes = URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .map_err(|_| "id_token mal formado")?;
    let claims: IdClaims = serde_json::from_slice(&bytes).map_err(|e| format!("id_token: {e}"))?;
    if !GOOGLE_ISSUERS.contains(&claims.iss.as_str()) {
        return Err(format!("id_token de otro emisor: {}", claims.iss));
    }
    if claims.aud != cfg.client_id {
        return Err("id_token para otro cliente".into());
    }
    if claims.exp <= now {
        return Err("id_token vencido".into());
    }
    if claims.sub.is_empty() {
        return Err("id_token sin sub".into());
    }
    Ok(claims)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cfg() -> GoogleConfig {
        GoogleConfig {
            client_id: "cliente".into(),
            client_secret: "secreto".into(),
            redirect_uri: "http://localhost/cb".into(),
            auth_url: String::new(),
            token_url: String::new(),
        }
    }

    fn token(claims: serde_json::Value) -> String {
        let enc = |v: serde_json::Value| URL_SAFE_NO_PAD.encode(v.to_string());
        format!("{}.{}.firma", enc(json!({ "alg": "RS256" })), enc(claims))
    }

    fn claims() -> serde_json::Value {
        json!({ "iss": "https://accounts.google.com", "aud": "cliente", "exp": 2000,
                "sub": "123", "email": "a@example.com", "email_verified": true })
    }

    #[test]
    fn accepts_a_valid_token() {
        let c = parse_id_token(&token(claims()), &cfg(), 1000).unwrap();
        assert_eq!(c.sub, "123");
        assert!(c.email_verified);
    }

    #[test]
    fn rejects_wrong_issuer_audience_expiry_or_shape() {
        for (field, value) in [
            ("iss", json!("https://evil.example.com")),
            ("aud", json!("otro-cliente")),
            ("exp", json!(1000)),
            ("sub", json!("")),
        ] {
            let mut c = claims();
            c[field] = value;
            assert!(parse_id_token(&token(c), &cfg(), 1000).is_err(), "{field}");
        }
        assert!(parse_id_token("sin-puntos", &cfg(), 1000).is_err());
    }

    #[test]
    fn pkce_challenge_is_s256() {
        // Calculado aparte con Node:
        // crypto.createHash("sha256").update(v).digest("base64url")
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r7wW1gFWO7JXk"),
            "TKsYUtqBtx100Jim-bYxuPN25j9L2lJWiYCry6nukG8"
        );
    }
}
