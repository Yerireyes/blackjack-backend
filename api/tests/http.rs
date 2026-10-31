//! Tests de punta a punta de la API contra PostgreSQL. Cada test recibe una base
//! nueva con las migraciones aplicadas (`#[sqlx::test]`).

use std::collections::HashMap;
use std::net::SocketAddr;

use api::config::{Config, GoogleConfig};
use api::state::AppState;
use api::{app, RateLimits};
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, Request, StatusCode};
use axum::routing::post;
use axum::{Form, Json, Router};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tower::ServiceExt;

const FRONT: &str = "http://localhost:5180";
const CLIENT_ID: &str = "cliente-de-prueba";
const CLIENT_SECRET: &str = "secreto-de-prueba";

fn config(analyze_burst: u32) -> Config {
    Config {
        database_url: String::new(),
        jwt_secret: b"secreto-de-prueba-con-al-menos-32-bytes".to_vec(),
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        frontend_origin: FRONT.into(),
        cookie_secure: false,
        analyze_replenish_ms: 60_000,
        analyze_burst,
        auth_burst: 1000,
        play_burst: 1000,
        max_rooms: 200,
        trusted_proxies: vec!["127.0.0.1".parse().unwrap(), "::1".parse().unwrap()],
        google: None,
        daily_seed_secret: None,
    }
}

fn test_app(pool: PgPool, analyze_burst: u32) -> Router {
    let cfg = config(analyze_burst);
    app(AppState::new(pool, &cfg), &cfg, &RateLimits::new(&cfg))
}

/// App con Google habilitado contra el servidor de tokens simulado.
fn google_app(pool: PgPool, token_url: String) -> Router {
    let mut cfg = config(10);
    cfg.google = Some(GoogleConfig {
        client_id: CLIENT_ID.into(),
        client_secret: CLIENT_SECRET.into(),
        redirect_uri: format!("{FRONT}/api/v1/auth/google/callback"),
        auth_url: api::config::GOOGLE_AUTH_URL.into(),
        token_url,
    });
    app(AppState::new(pool, &cfg), &cfg, &RateLimits::new(&cfg))
}

/// Endpoint de tokens de Google simulado. El `code` lleva la identidad que hay
/// que devolver y el `code_challenge` que vio Google: `sub|email|verificado|challenge`.
/// Comprueba PKCE y las credenciales del cliente como Google.
async fn mock_google() -> String {
    async fn token(Form(f): Form<HashMap<String, String>>) -> (StatusCode, Json<Value>) {
        let invalid = (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "invalid_grant" })),
        );
        let parts: Vec<&str> = f["code"].split('|').collect();
        let [sub, email, verified, challenge] = parts[..] else {
            return invalid;
        };
        let pkce = URL_SAFE_NO_PAD.encode(Sha256::digest(f["code_verifier"].as_bytes()));
        if pkce != challenge
            || f["client_id"] != CLIENT_ID
            || f["client_secret"] != CLIENT_SECRET
            || f["grant_type"] != "authorization_code"
            || !f["redirect_uri"].ends_with("/api/v1/auth/google/callback")
        {
            return invalid;
        }
        let enc = |v: Value| URL_SAFE_NO_PAD.encode(v.to_string());
        let claims = json!({
            "iss": "https://accounts.google.com", "aud": CLIENT_ID,
            "exp": time::OffsetDateTime::now_utc().unix_timestamp() + 3600,
            "sub": sub, "email": email, "email_verified": verified == "1",
        });
        let id_token = format!("{}.{}.firma", enc(json!({ "alg": "RS256" })), enc(claims));
        (
            StatusCode::OK,
            Json(json!({ "access_token": "x", "id_token": id_token, "token_type": "Bearer" })),
        )
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/token", post(token)))
            .await
            .unwrap()
    });
    format!("http://{addr}/token")
}

struct Res {
    status: StatusCode,
    /// "bj_session=<token>" si la respuesta la pone (o la borra).
    cookie: Option<String>,
    /// Todas las cookies de Set-Cookie como "nombre=valor".
    cookies: Vec<String>,
    location: Option<String>,
    body: Value,
}

async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
    cookie: Option<&str>,
) -> Res {
    let mut req = Request::builder().method(method).uri(uri);
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    let req = match body {
        Some(b) => req
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(b.to_string())),
        None => req.body(Body::empty()),
    }
    .unwrap();
    send_from(app, req, SocketAddr::from(([127, 0, 0, 1], 40000))).await
}

/// Manda la petición como si llegara por TCP desde `peer`.
async fn send_from(app: &Router, mut req: Request<Body>, peer: SocketAddr) -> Res {
    // Lo que pone axum::serve con into_make_service_with_connect_info (límite por IP).
    req.extensions_mut().insert(ConnectInfo(peer));
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    // "nombre=valor" de cada Set-Cookie, listo para mandarlo en Cookie.
    let cookies: Vec<String> = res
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_string())
        .collect();
    let cookie = cookies
        .iter()
        .find(|c| c.starts_with("bj_session="))
        .cloned();
    let location = res
        .headers()
        .get(header::LOCATION)
        .map(|v| v.to_str().unwrap().to_string());
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    Res {
        status,
        cookie,
        cookies,
        location,
        body,
    }
}

fn rules() -> Value {
    json!({
        "decks": 6, "dealerHitsSoft17": false, "doubleAfterSplit": true, "doubleOn": "any",
        "maxSplitHands": 4, "resplitAces": false, "hitSplitAces": false, "surrender": "late",
        "dealerPeeks": true, "blackjackPays": 1.5, "penetration": 0.75
    })
}

async fn register_named(app: &Router, email: &str, name: &str) -> Res {
    send(
        app,
        "POST",
        "/api/v1/auth/register",
        Some(json!({ "email": email, "password": "contraseña-segura", "displayName": name })),
        None,
    )
    .await
}

/// Registra con el nombre de la parte local del email y devuelve la cookie.
async fn register(app: &Router, email: &str) -> String {
    let res = register_named(app, email, email.split('@').next().unwrap()).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    res.cookie.expect("cookie de sesión")
}

/// Ida a Google como el navegador: devuelve la cookie `bj_oauth`, el `state` y el
/// `code_challenge` de la redirección.
async fn google_start(app: &Router, link: bool, session: Option<&str>) -> (String, String, String) {
    let uri = if link {
        "/api/v1/auth/google/start?link=1"
    } else {
        "/api/v1/auth/google/start"
    };
    let res = send(app, "GET", uri, None, session).await;
    assert_eq!(res.status, StatusCode::SEE_OTHER);
    let location = res.location.unwrap();
    assert!(
        location.starts_with(api::config::GOOGLE_AUTH_URL),
        "{location}"
    );
    let q: HashMap<String, String> = reqwest::Url::parse(&location)
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect();
    assert_eq!(q["client_id"], CLIENT_ID);
    assert_eq!(q["code_challenge_method"], "S256");
    assert_eq!(q["scope"], "openid email profile");
    let oauth = res
        .cookies
        .into_iter()
        .find(|c| c.starts_with("bj_oauth=") && c.len() > "bj_oauth=".len())
        .expect("cookie bj_oauth");
    (oauth, q["state"].clone(), q["code_challenge"].clone())
}

/// Vuelta de Google al callback con las cookies dadas (separadas por "; ").
async fn google_callback(app: &Router, params: &[(&str, &str)], cookies: &str) -> Res {
    let url =
        reqwest::Url::parse_with_params("http://x/api/v1/auth/google/callback", params).unwrap();
    let uri = format!("{}?{}", url.path(), url.query().unwrap());
    send(
        app,
        "GET",
        &uri,
        None,
        Some(cookies).filter(|c| !c.is_empty()),
    )
    .await
}

/// `POST /auth/google/link-intent` con la contraseña de `register`.
async fn link_intent(app: &Router, session: &str, password: &str) -> Res {
    send(
        app,
        "POST",
        "/api/v1/auth/google/link-intent",
        Some(json!({ "password": password })),
        Some(session),
    )
    .await
}

/// Flujo completo con la identidad `sub|email|verificado`. `start_session` y
/// `callback_session` son la cookie de sesión al ir y al volver. Para vincular,
/// primero confirma la contraseña (`link-intent`).
async fn google_flow(
    app: &Router,
    link: bool,
    start_session: Option<&str>,
    callback_session: Option<&str>,
    identity: &str,
) -> Res {
    if let (true, Some(session)) = (link, start_session) {
        let res = link_intent(app, session, "contraseña-segura").await;
        assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.body);
    }
    let (oauth, state, challenge) = google_start(app, link, start_session).await;
    let code = format!("{identity}|{challenge}");
    let cookies = match callback_session {
        Some(s) => format!("{oauth}; {s}"),
        None => oauth,
    };
    google_callback(app, &[("code", &code), ("state", &state)], &cookies).await
}

/// La redirección al front (lo que sigue a "?").
fn front_result(res: &Res) -> &str {
    assert_eq!(res.status, StatusCode::SEE_OTHER, "{}", res.body);
    let location = res.location.as_deref().unwrap();
    location
        .strip_prefix(&format!("{FRONT}/?"))
        .unwrap_or_else(|| panic!("redirección inesperada: {location}"))
}

async fn me(app: &Router, cookie: &str) -> Value {
    let res = send(app, "GET", "/api/v1/auth/me", None, Some(cookie)).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    res.body
}

#[sqlx::test(migrations = "./migrations")]
async fn analyze(pool: PgPool) {
    let app = test_app(pool, 10);
    let res = send(&app, "GET", "/api/v1/health", None, None).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["database"], "ok");
    let res = send(&app, "GET", "/api/v1/nada", None, None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(res.body["code"], "not_found");
}

#[sqlx::test(migrations = "./migrations")]
async fn analyze_uses_the_engine_contract(pool: PgPool) {
    let app = test_app(pool, 10);
    let req = json!({
        "version": 4, "rules": rules(),
        "shoe": [24, 24, 24, 24, 24, 23, 24, 24, 24, 94],
        "hand": [9, 5], "dealerUp": 9
    });
    let res = send(&app, "POST", "/api/v1/analyze/hand", Some(req), None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["version"], 4);
    assert_eq!(res.body["best"], "surrender");
    assert_eq!(res.body["actions"].as_array().unwrap().len(), 4);
    assert!(res.body["computeMs"].as_f64().unwrap() >= 0.0);

    let req = json!({ "version": 1, "rules": rules(), "shoe": [24, 24, 24, 24, 24, 23, 24, 24, 24, 96], "up": 5 });
    let res = send(&app, "POST", "/api/v1/analyze/dealer", Some(req), None).await;
    assert_eq!(res.status, StatusCode::OK);
    assert!((res.body["dealer"]["pBust"].as_f64().unwrap() - 0.422842).abs() < 1e-6);

    // Entrada imposible para el motor → 422 con el mensaje del motor.
    let req = json!({ "version": 1, "rules": rules(), "shoe": [0, 0, 0, 0, 0, 0, 0, 0, 0, 3], "hand": [9, 5], "dealerUp": 9 });
    let res = send(&app, "POST", "/api/v1/analyze/hand", Some(req), None).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.body["code"], "engine_error");
    assert!(res.body["message"].as_str().unwrap().contains("zapato"));

    // JSON que no cumple el contrato → error en formato { code, message }.
    let res = send(
        &app,
        "POST",
        "/api/v1/analyze/hand",
        Some(json!({ "hand": [12] })),
        None,
    )
    .await;
    assert!(res.status.is_client_error());
    assert_eq!(res.body["code"], "invalid_json");
}

#[sqlx::test(migrations = "./migrations")]
async fn analyze_is_rate_limited_per_ip(pool: PgPool) {
    let app = test_app(pool, 2);
    let req = json!({ "version": 1, "rules": rules(), "shoe": [24, 24, 24, 24, 24, 23, 24, 24, 24, 96], "up": 5 });
    for _ in 0..2 {
        let res = send(
            &app,
            "POST",
            "/api/v1/analyze/dealer",
            Some(req.clone()),
            None,
        )
        .await;
        assert_eq!(res.status, StatusCode::OK);
    }
    let res = send(&app, "POST", "/api/v1/analyze/dealer", Some(req), None).await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(res.body["code"], "rate_limited");
}

/// Detrás del reverse proxy, el límite va por la IP real aunque el cliente mande un
/// `X-Forwarded-For` falso: el proxy agrega la IP real a la derecha.
#[sqlx::test(migrations = "./migrations")]
async fn rate_limit_uses_real_ip_behind_proxy(pool: PgPool) {
    let app = test_app(pool, 1);
    let proxy = SocketAddr::from(([127, 0, 0, 1], 40000));
    let analyze = |xff: &str| {
        let body = json!({ "version": 1, "rules": rules(), "shoe": [24, 24, 24, 24, 24, 23, 24, 24, 24, 96], "up": 5 });
        Request::post("/api/v1/analyze/dealer")
            .header(header::CONTENT_TYPE, "application/json")
            .header("x-forwarded-for", xff)
            .body(Body::from(body.to_string()))
            .unwrap()
    };

    // La ráfaga es 1: la segunda petición de 203.0.113.9 se corta aunque cambie la IP falsa.
    let res = send_from(&app, analyze("1.2.3.4, 203.0.113.9"), proxy).await;
    assert_eq!(res.status, StatusCode::OK);
    let res = send_from(&app, analyze("5.6.7.8, 203.0.113.9"), proxy).await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS);

    // Otro cliente detrás del mismo proxy tiene su propio límite...
    let res = send_from(&app, analyze("203.0.113.10"), proxy).await;
    assert_eq!(res.status, StatusCode::OK);

    // ...y la IP falsa no se gastó: es otra clave.
    let res = send_from(&app, analyze("1.2.3.4"), proxy).await;
    assert_eq!(res.status, StatusCode::OK);

    // Sin pasar por el proxy, la cabecera se ignora y cuenta la IP de la conexión.
    let direct = SocketAddr::from(([198, 51, 100, 7], 5000));
    let res = send_from(&app, analyze("203.0.113.11"), direct).await;
    assert_eq!(res.status, StatusCode::OK);
    let res = send_from(&app, analyze("203.0.113.12"), direct).await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS);
}

#[sqlx::test(migrations = "./migrations")]
async fn auth_flow(pool: PgPool) {
    let app = test_app(pool, 10);
    let cookie = register(&app, "Ana@Example.com").await;
    assert!(cookie.starts_with("bj_session="));

    // Mismo email con otras mayúsculas → 409.
    let res = register_named(&app, "ana@example.com", "Otra Ana").await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["code"], "email_taken");

    // Validaciones.
    for body in [
        json!({ "email": "no-es-email", "password": "contraseña-segura", "displayName": "Beto" }),
        json!({ "email": "b@example.com", "password": "corta", "displayName": "Beto" }),
    ] {
        let res = send(&app, "POST", "/api/v1/auth/register", Some(body), None).await;
        assert_eq!(res.status, StatusCode::BAD_REQUEST);
    }

    let res = send(
        &app,
        "POST",
        "/api/v1/auth/login",
        Some(json!({ "email": "ana@example.com", "password": "incorrecta" })),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    assert_eq!(res.body["code"], "invalid_credentials");
    let res = send(
        &app,
        "POST",
        "/api/v1/auth/login",
        Some(json!({ "email": "nadie@example.com", "password": "incorrecta" })),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);

    let res = send(
        &app,
        "POST",
        "/api/v1/auth/login",
        Some(json!({ "email": "ANA@example.com", "password": "contraseña-segura" })),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::OK);
    let cookie = res.cookie.unwrap();

    let res = send(&app, "GET", "/api/v1/auth/me", None, Some(&cookie)).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["email"], "Ana@Example.com");

    let res = send(&app, "POST", "/api/v1/auth/logout", None, Some(&cookie)).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    assert_eq!(res.cookie.as_deref(), Some("bj_session="));

    // El token copiado antes del logout ya no vale.
    let res = send(&app, "GET", "/api/v1/auth/me", None, Some(&cookie)).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    let res = send(&app, "POST", "/api/v1/auth/logout", None, Some(&cookie)).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);

    let res = send(&app, "GET", "/api/v1/auth/me", None, None).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    let res = send(
        &app,
        "GET",
        "/api/v1/auth/me",
        None,
        Some("bj_session=token.falso.x"),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
}

async fn me_status(app: &Router, cookie: &str) -> StatusCode {
    send(app, "GET", "/api/v1/auth/me", None, Some(cookie))
        .await
        .status
}

async fn login(app: &Router, email: &str) -> String {
    let res = send(
        app,
        "POST",
        "/api/v1/auth/login",
        Some(json!({ "email": email, "password": "contraseña-segura" })),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::OK);
    res.cookie.unwrap()
}

/// Cada login es una sesión aparte: cerrar una no toca las demás, y `logout-all`
/// las cierra todas.
#[sqlx::test(migrations = "./migrations")]
async fn sessions_are_revocable(pool: PgPool) {
    let app = test_app(pool.clone(), 10);
    let phone = register(&app, "ana@example.com").await;
    let laptop = login(&app, "ana@example.com").await;
    let tablet = login(&app, "ana@example.com").await;
    let other_user = register_named(&app, "beto@example.com", "Beto")
        .await
        .cookie
        .unwrap();

    // Logout en el teléfono: las otras dos siguen.
    let res = send(&app, "POST", "/api/v1/auth/logout", None, Some(&phone)).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    assert_eq!(me_status(&app, &phone).await, StatusCode::UNAUTHORIZED);
    assert_eq!(me_status(&app, &laptop).await, StatusCode::OK);

    // logout-all desde la laptop: cierra también la tablet, no la de otro usuario.
    let res = send(&app, "POST", "/api/v1/auth/logout-all", None, Some(&laptop)).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    assert_eq!(res.cookie.as_deref(), Some("bj_session="));
    for cookie in [&laptop, &tablet] {
        assert_eq!(me_status(&app, cookie).await, StatusCode::UNAUTHORIZED);
        let res = send(
            &app,
            "PATCH",
            "/api/v1/me",
            Some(json!({ "displayName": "Ana Robada" })),
            Some(cookie),
        )
        .await;
        assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(me_status(&app, &other_user).await, StatusCode::OK);

    // Volver a entrar funciona.
    let again = login(&app, "ana@example.com").await;
    assert_eq!(me_status(&app, &again).await, StatusCode::OK);

    // La limpieza borra las revocadas hace más de un día y deja las vivas.
    sqlx::query("UPDATE auth_sessions SET revoked_at = now() - interval '2 days' WHERE revoked_at IS NOT NULL")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(api::auth::purge_sessions(&pool).await.unwrap(), 3);
    assert_eq!(me_status(&app, &again).await, StatusCode::OK);
    assert_eq!(me_status(&app, &other_user).await, StatusCode::OK);
}

/// Un JWT bien firmado pero sin fila en `auth_sessions` (por ejemplo, uno emitido
/// antes de las sesiones revocables) no vale.
#[sqlx::test(migrations = "./migrations")]
async fn token_without_session_row_is_rejected(pool: PgPool) {
    let app = test_app(pool.clone(), 10);
    let cookie = register(&app, "ana@example.com").await;
    sqlx::query("DELETE FROM auth_sessions")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(me_status(&app, &cookie).await, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = "./migrations")]
async fn display_names(pool: PgPool) {
    let app = test_app(pool, 10);

    // "José" con la tilde como carácter combinado se guarda en NFC.
    let res = register_named(&app, "jose@example.com", "  Jose\u{301} ").await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert_eq!(res.body["displayName"], "José");
    assert_eq!(res.body["needsDisplayName"], false);
    assert_eq!(res.body["hasPassword"], true);
    assert_eq!(res.body["providers"], json!([]));
    let jose = res.cookie.unwrap();

    // /auth/me devuelve exactamente estos campos: nunca el hash.
    let body = me(&app, &jose).await;
    let mut keys: Vec<&str> = body
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "createdAt",
            "displayName",
            "email",
            "hasPassword",
            "id",
            "needsDisplayName",
            "providers"
        ]
    );

    // Duplicado sin distinguir mayúsculas (también con tildes) → 409.
    let res = register_named(&app, "otro@example.com", "JOSÉ").await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["code"], "display_name_taken");

    for bad in [
        "ab",
        "dos  espacios",
        "\u{410}dmin",
        "Admín",
        "c-a s_a",
        "con.punto",
    ] {
        let res = register_named(&app, "otro@example.com", bad).await;
        assert_eq!(res.status, StatusCode::BAD_REQUEST, "{bad:?}");
        assert_eq!(res.body["code"], "display_name_invalid", "{bad:?}");
    }
    let res = send(
        &app,
        "POST",
        "/api/v1/auth/register",
        Some(json!({ "email": "otro@example.com", "password": "contraseña-segura" })),
        None,
    )
    .await;
    assert_eq!(res.body["code"], "invalid_json");

    // PATCH /me.
    let res = send(
        &app,
        "PATCH",
        "/api/v1/me",
        Some(json!({ "displayName": "Nadie" })),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    let beto = register(&app, "beto@example.com").await;
    let res = send(
        &app,
        "PATCH",
        "/api/v1/me",
        Some(json!({ "displayName": "josé" })),
        Some(&beto),
    )
    .await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["code"], "display_name_taken");
    let res = send(
        &app,
        "PATCH",
        "/api/v1/me",
        Some(json!({ "displayName": "admin" })),
        Some(&beto),
    )
    .await;
    assert_eq!(res.body["code"], "display_name_invalid");
    let res = send(
        &app,
        "PATCH",
        "/api/v1/me",
        Some(json!({ "displayName": "Beto Pérez" })),
        Some(&beto),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["displayName"], "Beto Pérez");
    // Cambiar solo las mayúsculas del propio nombre está permitido.
    let res = send(
        &app,
        "PATCH",
        "/api/v1/me",
        Some(json!({ "displayName": "BETO PÉREZ" })),
        Some(&beto),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
}

#[sqlx::test(migrations = "./migrations")]
async fn google_disabled_without_credentials(pool: PgPool) {
    let app = test_app(pool, 10);
    let res = send(&app, "GET", "/api/v1/auth/providers", None, None).await;
    assert_eq!(res.body, json!({ "google": false }));
    for (uri, expected) in [
        (
            "/api/v1/auth/google/start",
            "login=error&code=google_disabled",
        ),
        (
            "/api/v1/auth/google/start?link=1",
            "link=error&code=google_disabled",
        ),
        (
            "/api/v1/auth/google/callback?code=x&state=y",
            "login=error&code=google_disabled",
        ),
    ] {
        let res = send(&app, "GET", uri, None, None).await;
        assert_eq!(front_result(&res), expected, "{uri}");
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn google_state_and_pkce(pool: PgPool) {
    let app = google_app(pool, mock_google().await);
    let res = send(&app, "GET", "/api/v1/auth/providers", None, None).await;
    assert_eq!(res.body, json!({ "google": true }));

    // `state` distinto al de la cookie → error, y la cookie se borra.
    let (oauth, _, challenge) = google_start(&app, false, None).await;
    let code = format!("g1|a@example.com|1|{challenge}");
    let res = google_callback(&app, &[("code", &code), ("state", "otro")], &oauth).await;
    assert_eq!(front_result(&res), "login=error&code=invalid_state");
    assert!(res.cookies.iter().any(|c| c == "bj_oauth="));
    assert!(res.cookie.is_none());

    // Sin la cookie de `start` → error.
    let (_, state, challenge) = google_start(&app, false, None).await;
    let code = format!("g1|a@example.com|1|{challenge}");
    let res = google_callback(&app, &[("code", &code), ("state", &state)], "").await;
    assert_eq!(front_result(&res), "login=error&code=invalid_state");

    // Cookie alterada (no se puede descifrar) → error.
    let (oauth, state, challenge) = google_start(&app, false, None).await;
    let code = format!("g1|a@example.com|1|{challenge}");
    let res = google_callback(
        &app,
        &[("code", &code), ("state", &state)],
        &format!("{oauth}x"),
    )
    .await;
    assert_eq!(front_result(&res), "login=error&code=invalid_state");

    // El usuario cancela en Google.
    let (oauth, state, _) = google_start(&app, false, None).await;
    let res = google_callback(
        &app,
        &[("error", "access_denied"), ("state", &state)],
        &oauth,
    )
    .await;
    assert_eq!(front_result(&res), "login=error&code=cancelled");

    // `code_verifier` que no corresponde al `code_challenge` → Google lo rechaza.
    let (oauth, state, _) = google_start(&app, false, None).await;
    let code = "g1|a@example.com|1|otro-challenge";
    let res = google_callback(&app, &[("code", code), ("state", &state)], &oauth).await;
    assert_eq!(front_result(&res), "login=error&code=google_error");
    assert!(res.cookie.is_none());
}

#[sqlx::test(migrations = "./migrations")]
async fn google_login_creates_account_with_provisional_name(pool: PgPool) {
    let app = google_app(pool, mock_google().await);

    let res = google_flow(&app, false, None, None, "g1|nuevo@example.com|1").await;
    assert_eq!(front_result(&res), "login=ok");
    let cookie = res.cookie.expect("cookie de sesión");
    let body = me(&app, &cookie).await;
    assert_eq!(body["email"], "nuevo@example.com");
    assert_eq!(body["needsDisplayName"], true);
    assert_eq!(body["hasPassword"], false);
    assert_eq!(body["providers"], json!(["google"]));
    assert!(body["displayName"].as_str().unwrap().starts_with("Jugador"));
    let id = body["id"].clone();

    // Sin contraseña: el login por contraseña da el error genérico.
    let res = send(
        &app,
        "POST",
        "/api/v1/auth/login",
        Some(json!({ "email": "nuevo@example.com", "password": "cualquiera" })),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    assert_eq!(res.body["code"], "invalid_credentials");

    // Elegir nombre confirma el provisional.
    let res = send(
        &app,
        "PATCH",
        "/api/v1/me",
        Some(json!({ "displayName": "Nuevo" })),
        Some(&cookie),
    )
    .await;
    assert_eq!(res.body["needsDisplayName"], false);

    // Volver a entrar con la misma identidad (aunque cambie el email) → misma cuenta.
    let res = google_flow(&app, false, None, None, "g1|otro-email@example.com|1").await;
    assert_eq!(front_result(&res), "login=ok");
    assert_eq!(me(&app, &res.cookie.unwrap()).await["id"], id);

    // Email sin verificar → no se crea la cuenta.
    let res = google_flow(&app, false, None, None, "g2|sin-verificar@example.com|0").await;
    assert_eq!(front_result(&res), "login=error&code=email_unverified");
    assert!(res.cookie.is_none());
}

#[sqlx::test(migrations = "./migrations")]
async fn google_links_existing_accounts_only_explicitly(pool: PgPool) {
    let app = google_app(pool, mock_google().await);
    let ana = register(&app, "ana@example.com").await;
    let ana_id = me(&app, &ana).await["id"].clone();

    // Google con el email de una cuenta existente (aunque esté verificado) no se
    // vincula solo: el email de la cuenta con contraseña nunca se verificó.
    let res = google_flow(&app, false, None, None, "g-ana|ANA@example.com|1").await;
    assert_eq!(front_result(&res), "login=error&code=account_exists");
    assert!(res.cookie.is_none());
    assert_eq!(me(&app, &ana).await["providers"], json!([]));

    // Vincular sin sesión → error.
    let res = send(&app, "GET", "/api/v1/auth/google/start?link=1", None, None).await;
    assert_eq!(front_result(&res), "link=error&code=unauthorized");

    // Si la sesión cambia entre la ida y la vuelta → error.
    let beto = register(&app, "beto@example.com").await;
    let res = google_flow(&app, true, Some(&ana), Some(&beto), "g-ana|ana@gmail.com|1").await;
    assert_eq!(front_result(&res), "link=error&code=unauthorized");

    // Vinculación explícita desde la cuenta de Ana.
    let res = google_flow(&app, true, Some(&ana), Some(&ana), "g-ana|ana@gmail.com|1").await;
    assert_eq!(front_result(&res), "link=ok");
    assert!(res.cookie.is_none(), "la sesión no cambia");
    let body = me(&app, &ana).await;
    assert_eq!(body["providers"], json!(["google"]));
    assert_eq!(body["hasPassword"], true);

    // Desde ahora Google entra en la cuenta de Ana.
    let res = google_flow(&app, false, None, None, "g-ana|ana@gmail.com|1").await;
    assert_eq!(front_result(&res), "login=ok");
    assert_eq!(me(&app, &res.cookie.unwrap()).await["id"], ana_id);

    // Con Google ya vinculado, link-intent lo dice antes de ir a Google.
    let res = link_intent(&app, &ana, "contraseña-segura").await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["code"], "already_linked");

    // La identidad de Ana no se puede vincular a otra cuenta.
    let res = google_flow(
        &app,
        true,
        Some(&beto),
        Some(&beto),
        "g-ana|ana@gmail.com|1",
    )
    .await;
    assert_eq!(front_result(&res), "link=error&code=identity_in_use");
    assert_eq!(me(&app, &beto).await["providers"], json!([]));
}

/// Vincular exige confirmar la contraseña justo antes: un token de sesión solo no basta.
#[sqlx::test(migrations = "./migrations")]
async fn google_link_requires_password(pool: PgPool) {
    let app = google_app(pool.clone(), mock_google().await);
    let ana = register(&app, "ana@example.com").await;

    // Sin link-intent → reauth_required, sin pasar por Google.
    let res = send(
        &app,
        "GET",
        "/api/v1/auth/google/start?link=1",
        None,
        Some(&ana),
    )
    .await;
    assert_eq!(front_result(&res), "link=error&code=reauth_required");

    // Contraseña incorrecta → 403 (no 401: la sesión sigue viva).
    let res = link_intent(&app, &ana, "incorrecta").await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    assert_eq!(res.body["code"], "invalid_password");
    assert_eq!(me_status(&app, &ana).await, StatusCode::OK);

    // La prueba es de un solo uso: si cancela en Google, hay que pedirla de nuevo.
    let res = link_intent(&app, &ana, "contraseña-segura").await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    let (oauth, state, _) = google_start(&app, true, Some(&ana)).await;
    let res = google_callback(
        &app,
        &[("error", "access_denied"), ("state", &state)],
        &format!("{oauth}; {ana}"),
    )
    .await;
    assert_eq!(front_result(&res), "link=error&code=cancelled");
    let res = send(
        &app,
        "GET",
        "/api/v1/auth/google/start?link=1",
        None,
        Some(&ana),
    )
    .await;
    assert_eq!(front_result(&res), "link=error&code=reauth_required");

    // Es de la sesión que la pidió, no de las demás sesiones del usuario.
    let laptop = login(&app, "ana@example.com").await;
    let res = link_intent(&app, &ana, "contraseña-segura").await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    let res = send(
        &app,
        "GET",
        "/api/v1/auth/google/start?link=1",
        None,
        Some(&laptop),
    )
    .await;
    assert_eq!(front_result(&res), "link=error&code=reauth_required");

    // Vencida → reauth_required.
    sqlx::query("UPDATE auth_sessions SET link_intent_until = now() - interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();
    let res = send(
        &app,
        "GET",
        "/api/v1/auth/google/start?link=1",
        None,
        Some(&ana),
    )
    .await;
    assert_eq!(front_result(&res), "link=error&code=reauth_required");

    // Cuenta creada con Google: no tiene contraseña que confirmar.
    let res = google_flow(&app, false, None, None, "g1|nuevo@example.com|1").await;
    let nuevo = res.cookie.unwrap();
    let res = link_intent(&app, &nuevo, "cualquiera").await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["code"], "no_password");
}

#[sqlx::test(migrations = "./migrations")]
async fn google_unlink(pool: PgPool) {
    let app = google_app(pool, mock_google().await);
    let ana = register(&app, "ana@example.com").await;
    let unlink = |session: String, password: &'static str| {
        let app = app.clone();
        async move {
            send(
                &app,
                "POST",
                "/api/v1/auth/google/unlink",
                Some(json!({ "password": password })),
                Some(&session),
            )
            .await
        }
    };

    // Sin Google vinculado → 409 not_linked.
    let res = unlink(ana.clone(), "contraseña-segura").await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["code"], "not_linked");

    let res = google_flow(&app, true, Some(&ana), Some(&ana), "g-ana|ana@gmail.com|1").await;
    assert_eq!(front_result(&res), "link=ok");
    // Alguien entra con ese Google (otra sesión de la misma cuenta).
    let res = google_flow(&app, false, None, None, "g-ana|ana@gmail.com|1").await;
    let via_google = res.cookie.unwrap();

    // Contraseña incorrecta → 403 y nada cambia.
    let res = unlink(ana.clone(), "incorrecta").await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    assert_eq!(res.body["code"], "invalid_password");
    assert_eq!(me(&app, &ana).await["providers"], json!(["google"]));

    // Desvincular devuelve el Me y revoca las demás sesiones, no la actual.
    let res = unlink(ana.clone(), "contraseña-segura").await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["providers"], json!([]));
    assert_eq!(me_status(&app, &ana).await, StatusCode::OK);
    assert_eq!(me_status(&app, &via_google).await, StatusCode::UNAUTHORIZED);

    // Ese Google ya no entra en la cuenta de Ana: crearía otra (el email es distinto).
    let res = google_flow(&app, false, None, None, "g-ana|ana@gmail.com|1").await;
    assert_eq!(front_result(&res), "login=ok");
    let otra = res.cookie.unwrap();
    assert_ne!(me(&app, &otra).await["id"], me(&app, &ana).await["id"]);

    // Una cuenta sin contraseña no puede desvincular su único acceso.
    let res = unlink(otra.clone(), "cualquiera").await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["code"], "no_password");
    assert_eq!(me(&app, &otra).await["providers"], json!(["google"]));
}

#[sqlx::test(migrations = "./migrations")]
async fn sessions_hands_and_stats(pool: PgPool) {
    let app = test_app(pool, 10);
    let ana = register(&app, "ana@example.com").await;
    let beto = register(&app, "beto@example.com").await;

    let res = send(
        &app,
        "POST",
        "/api/v1/sessions",
        Some(json!({ "rules": rules(), "mode": "player" })),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);

    let mut bad_rules = rules();
    bad_rules["decks"] = json!(0);
    let res = send(
        &app,
        "POST",
        "/api/v1/sessions",
        Some(json!({ "rules": bad_rules, "mode": "player" })),
        Some(&ana),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);

    let res = send(
        &app,
        "POST",
        "/api/v1/sessions",
        Some(json!({ "rules": rules(), "mode": "player" })),
        Some(&ana),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert_eq!(res.body["rules"], rules());
    let id = res.body["id"].as_str().unwrap().to_string();

    let hands = [
        // Óptima, ganó.
        json!({ "playerCards": [9, 5], "dealerUp": 9, "dealerFinal": [9, 6, 9], "chosenAction": "hit", "bestAction": "hit",
                "evChosen": -0.534, "evBest": -0.534, "exact": true, "result": "win" }),
        // Se plantó con 16 vs 10 cuando convenía pedir: pierde 0.006 de EV.
        json!({ "playerCards": [9, 5], "dealerUp": 9, "dealerFinal": [9, 8], "chosenAction": "stand", "bestAction": "hit",
                "evChosen": -0.541, "evBest": -0.535, "exact": true, "result": "lose" }),
        // Dividió (aproximado), empate.
        json!({ "playerCards": [7, 7], "dealerUp": 9, "dealerFinal": [9, 7], "chosenAction": "split", "bestAction": "split",
                "evChosen": -0.45, "evBest": -0.45, "exact": false, "result": "push" }),
    ];
    for h in hands {
        let res = send(
            &app,
            "POST",
            &format!("/api/v1/sessions/{id}/hands"),
            Some(h),
            Some(&ana),
        )
        .await;
        assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    }

    let res = send(
        &app,
        "GET",
        &format!("/api/v1/sessions/{id}/stats"),
        None,
        Some(&ana),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK);
    let s = &res.body;
    assert_eq!(s["hands"], 3);
    assert_eq!(s["optimalDecisions"], 2);
    assert!((s["optimalPct"].as_f64().unwrap() - 200.0 / 3.0).abs() < 1e-9);
    assert!((s["evLost"].as_f64().unwrap() - 0.006).abs() < 1e-12);
    assert_eq!(s["approximateDecisions"], 1);
    assert_eq!(
        (s["wins"].clone(), s["pushes"].clone(), s["losses"].clone()),
        (json!(1), json!(1), json!(1))
    );

    // Otro usuario no ve la sesión.
    let res = send(
        &app,
        "GET",
        &format!("/api/v1/sessions/{id}/stats"),
        None,
        Some(&beto),
    )
    .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = send(&app, "GET", "/api/v1/sessions", None, Some(&beto)).await;
    assert_eq!(res.body.as_array().unwrap().len(), 0);
    let res = send(&app, "GET", "/api/v1/sessions", None, Some(&ana)).await;
    assert_eq!(res.body.as_array().unwrap().len(), 1);

    // Carta inválida.
    let res = send(
        &app,
        "POST",
        &format!("/api/v1/sessions/{id}/hands"),
        Some(json!({ "playerCards": [10, 5], "dealerUp": 9, "dealerFinal": [], "chosenAction": "hit", "bestAction": "hit",
                     "evChosen": 0.0, "evBest": 0.0, "exact": true, "result": "win" })),
        Some(&ana),
    )
    .await;
    assert!(res.status.is_client_error());

    // Sesión terminada: no admite más manos.
    let res = send(
        &app,
        "POST",
        &format!("/api/v1/sessions/{id}/end"),
        None,
        Some(&ana),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK);
    assert!(res.body["endedAt"].is_string());
    let res = send(
        &app,
        "POST",
        &format!("/api/v1/sessions/{id}/hands"),
        Some(json!({ "playerCards": [9, 5], "dealerUp": 9, "dealerFinal": [9, 8], "chosenAction": "hit", "bestAction": "hit",
                     "evChosen": 0.0, "evBest": 0.0, "exact": true, "result": "lose" })),
        Some(&ana),
    )
    .await;
    assert_eq!(res.status, StatusCode::CONFLICT);

    let res = send(
        &app,
        "GET",
        "/api/v1/sessions/no-es-uuid/stats",
        None,
        Some(&ana),
    )
    .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    assert_eq!(res.body["code"], "invalid_path");
}

// ---- Casual ----

use engine::game::{Face, GameShoe};

/// Crea una partida casual con el preset y devuelve su id.
async fn new_run(app: &Router, cookie: &str, preset: &str) -> String {
    let res = send(
        app,
        "POST",
        "/api/v1/casual/runs",
        Some(json!({ "preset": preset })),
        Some(cookie),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    res.body["id"].as_str().unwrap().to_string()
}

/// Pone estas cartas arriba del zapato de la partida (como `stack` del front).
async fn stack(pool: &PgPool, run: &str, decks: u8, top: &[Face]) {
    let shoe = serde_json::to_value(GameShoe::stacked(decks, top)).unwrap();
    sqlx::query("UPDATE casual_runs SET shoe = $2 WHERE id = $1::uuid")
        .bind(run)
        .bind(shoe)
        .execute(pool)
        .await
        .unwrap();
}

async fn set_balance(pool: &PgPool, run: &str, balance: i64) {
    sqlx::query("UPDATE casual_runs SET balance = $2 WHERE id = $1::uuid")
        .bind(run)
        .bind(balance)
        .execute(pool)
        .await
        .unwrap();
}

fn key() -> String {
    uuid::Uuid::new_v4().to_string()
}

async fn deal_round(app: &Router, cookie: &str, bet: i64, key: &str) -> Res {
    send(
        app,
        "POST",
        "/api/v1/play/rounds",
        Some(json!({ "kind": "casual", "bet": bet, "idempotencyKey": key })),
        Some(cookie),
    )
    .await
}

async fn act(app: &Router, cookie: &str, round: &str, action: &str, key: &str) -> Res {
    send(
        app,
        "POST",
        &format!("/api/v1/play/rounds/{round}/actions"),
        Some(json!({ "action": action, "idempotencyKey": key })),
        Some(cookie),
    )
    .await
}

fn shoe_total(view: &Value) -> u64 {
    view["shoeCounts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap())
        .sum()
}

async fn post_run(app: &Router, cookie: Option<&str>, body: Value) -> Res {
    send(app, "POST", "/api/v1/casual/runs", Some(body), cookie).await
}

#[sqlx::test(migrations = "./migrations")]
async fn casual_runs(pool: PgPool) {
    let app = test_app(pool.clone(), 10);
    let ana = register(&app, "ana@example.com").await;

    // Sin sesión → 401; preset y reglas a la vez (o ninguno) → 400.
    let res = post_run(&app, None, json!({ "preset": "vegas" })).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    for body in [json!({}), json!({ "preset": "vegas", "rules": rules() })] {
        let res = post_run(&app, Some(&ana), body).await;
        assert_eq!(res.status, StatusCode::BAD_REQUEST);
    }
    let res = send(&app, "GET", "/api/v1/casual/runs/active", None, Some(&ana)).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(res.body["code"], "no_active_run");

    let res = post_run(&app, Some(&ana), json!({ "preset": "sixToFive" })).await;
    assert_eq!(res.status, StatusCode::CREATED);
    assert_eq!(res.body["balance"], 500);
    assert_eq!(res.body["peak"], 500);
    assert_eq!(res.body["maxBet"], 500);
    assert_eq!(res.body["status"], "active");
    assert_eq!(res.body["preset"], "sixToFive");
    assert_eq!(res.body["rules"]["blackjackPays"], 1.2);
    assert!(
        res.body.get("shoe").is_none(),
        "el zapato nunca sale del servidor"
    );
    let run = res.body["id"].as_str().unwrap().to_string();

    // Una sola partida activa.
    let res = post_run(&app, Some(&ana), json!({ "preset": "vegas" })).await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["code"], "run_active");
    let res = send(&app, "GET", "/api/v1/casual/runs/active", None, Some(&ana)).await;
    assert_eq!(res.body["id"], run.as_str());

    // Retirarse cierra la partida y no se reabre.
    let uri = format!("/api/v1/casual/runs/{run}/retire");
    let res = send(&app, "POST", &uri, None, Some(&ana)).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["status"], "retired");
    assert_eq!(res.body["maxBet"], 0);
    assert!(res.body["endedAt"].is_string());
    let res = send(&app, "POST", &uri, None, Some(&ana)).await;
    assert_eq!(res.body["code"], "run_finished");

    // Reglas personalizadas inválidas → 422 del motor; válidas → sin preset.
    let mut bad = rules();
    bad["decks"] = json!(0);
    let res = post_run(&app, Some(&ana), json!({ "rules": bad })).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    let res = post_run(&app, Some(&ana), json!({ "rules": rules() })).await;
    assert_eq!(res.status, StatusCode::CREATED);
    assert!(res.body.get("preset").is_none());

    // Otro usuario no cierra la partida de Ana.
    let beto = register(&app, "beto@example.com").await;
    let res = send(&app, "POST", &uri, None, Some(&beto)).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);

    // Con el nombre provisional no se juega (el nombre sale en los rankings).
    sqlx::query("UPDATE users SET display_name_set = false WHERE email = 'beto@example.com'")
        .execute(&pool)
        .await
        .unwrap();
    let res = post_run(&app, Some(&beto), json!({ "preset": "vegas" })).await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["code"], "display_name_required");
}

#[sqlx::test(migrations = "./migrations")]
async fn casual_round_flow(pool: PgPool) {
    use Face::*;
    let app = test_app(pool.clone(), 10);
    let ana = register(&app, "ana@example.com").await;
    let run = new_run(&app, &ana, "vegas").await;

    // Jugador 10,8; dealer 6 + oculta 10 = 16 → pide un 5 → 21.
    stack(&pool, &run, 6, &[Ten, Six, Eight, Ten, Five]).await;
    let res = deal_round(&app, &ana, 50, &key()).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let view = res.body;
    let round = view["id"].as_str().unwrap().to_string();
    assert_eq!(view["kind"], "casual");
    assert_eq!(view["contextId"], run.as_str());
    assert_eq!(view["phase"], "player");
    assert_eq!(view["bet"], 50);
    assert_eq!(view["player"]["hands"][0]["cards"][0]["rank"], "10");
    assert_eq!(view["dealer"]["up"]["rank"], "6");
    assert_eq!(view["dealer"]["hole"], Value::Null, "la oculta no se ve");
    assert_eq!(
        view["allowed"],
        json!({ "hit": true, "stand": true, "double": true, "split": false, "surrender": true,
                "insurance": false })
    );
    assert_eq!(view["peekBlackjack"], false);
    assert_eq!(view["wallet"], json!({ "balance": 500, "maxBet": 500 }));
    // 312 − 3 visibles: la oculta cuenta como no vista.
    assert_eq!(shoe_total(&view), 309);
    assert!(view.get("result").is_none());

    // Una sola ronda activa; no se puede retirar con ella en juego.
    let res = deal_round(&app, &ana, 10, &key()).await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["code"], "round_active");
    let uri = format!("/api/v1/casual/runs/{run}/retire");
    let res = send(&app, "POST", &uri, None, Some(&ana)).await;
    assert_eq!(res.body["code"], "round_active");

    // Retomar tras desconectarse.
    let res = send(&app, "GET", "/api/v1/play/active", None, Some(&ana)).await;
    assert_eq!(res.body, view);

    // Acción no permitida (no es un par) → 409.
    let res = act(&app, &ana, &round, "split", &key()).await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["code"], "action_not_allowed");

    // Otro usuario no puede actuar en la ronda de Ana.
    let beto = register(&app, "beto@example.com").await;
    let res = act(&app, &beto, &round, "stand", &key()).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);

    let res = act(&app, &ana, &round, "stand", &key()).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let settled = res.body;
    assert_eq!(settled["phase"], "settled");
    assert_eq!(settled["dealer"]["hole"]["rank"], "10");
    assert_eq!(settled["dealer"]["hits"][0]["rank"], "5");
    assert_eq!(settled["player"]["hands"][0]["outcome"], "lose");
    assert_eq!(settled["player"]["hands"][0]["net"], -50);
    assert_eq!(settled["result"]["net"], -50);
    assert_eq!(settled["wallet"]["balance"], 450);
    assert_eq!(settled["allowed"]["stand"], false);
    assert_eq!(shoe_total(&settled), 312 - 5);

    let res = act(&app, &ana, &round, "hit", &key()).await;
    assert_eq!(res.body["code"], "no_active_round");
    let res = send(&app, "GET", "/api/v1/play/active", None, Some(&ana)).await;
    assert_eq!(res.body["code"], "no_active_round");
    let res = send(&app, "GET", "/api/v1/casual/runs/active", None, Some(&ana)).await;
    assert_eq!(res.body["balance"], 450);
    assert_eq!(res.body["hands"], 1);
    assert_eq!(res.body["peak"], 500);

    // Peek con blackjack del dealer (K visible, As oculto): termina en el reparto. Con
    // un As visible primero se ofrecería el seguro (ver `insurance_in_casual`).
    stack(&pool, &run, 6, &[Ten, K, Seven, Ace]).await;
    let res = deal_round(&app, &ana, 100, &key()).await;
    assert_eq!(res.body["phase"], "settled");
    assert_eq!(res.body["peekBlackjack"], true);
    assert_eq!(res.body["dealer"]["hole"]["rank"], "A");
    assert_eq!(res.body["result"]["net"], -100);
    assert_eq!(res.body["wallet"]["balance"], 350);

    // Blackjack del jugador paga 3:2.
    stack(&pool, &run, 6, &[Ace, Nine, K, Seven]).await;
    let res = deal_round(&app, &ana, 100, &key()).await;
    assert_eq!(res.body["player"]["hands"][0]["outcome"], "blackjack");
    assert_eq!(res.body["result"]["net"], 150);
    assert_eq!(res.body["wallet"]["balance"], 500);

    // Dividir y doblar después de dividir.
    stack(
        &pool,
        &run,
        6,
        &[Eight, Six, Eight, Ten, Three, Ten, Five, Two],
    )
    .await;
    let res = deal_round(&app, &ana, 100, &key()).await;
    let round = res.body["id"].as_str().unwrap().to_string();
    let res = act(&app, &ana, &round, "split", &key()).await;
    assert_eq!(res.body["player"]["hands"].as_array().unwrap().len(), 2);
    assert_eq!(res.body["player"]["active"], 0);
    // 8,3 = 11: dobla y saca un 10; la segunda mano recibe un 5 (8,5 = 13).
    let res = act(&app, &ana, &round, "double", &key()).await;
    assert_eq!(res.body["player"]["active"], 1);
    assert_eq!(res.body["player"]["hands"][0]["doubled"], true);
    let res = act(&app, &ana, &round, "stand", &key()).await;
    assert_eq!(res.body["phase"], "settled");
    // Dealer 6,10 pide un 2 → 18: la mano de 21 gana 200 y la de 13 pierde 100.
    assert_eq!(res.body["player"]["hands"][0]["net"], 200);
    assert_eq!(res.body["player"]["hands"][1]["net"], -100);
    assert_eq!(res.body["result"]["net"], 100);
    assert_eq!(res.body["wallet"]["balance"], 600);
    assert_eq!(res.body["wallet"]["maxBet"], 500);
}

#[sqlx::test(migrations = "./migrations")]
async fn casual_idempotency(pool: PgPool) {
    use Face::*;
    let app = test_app(pool.clone(), 10);
    let ana = register(&app, "ana@example.com").await;
    let run = new_run(&app, &ana, "vegas").await;

    // Misma clave al repartir → misma respuesta y un solo pago.
    stack(&pool, &run, 6, &[Ace, Nine, K, Seven]).await;
    let k = key();
    let first = deal_round(&app, &ana, 100, &k).await;
    let again = deal_round(&app, &ana, 100, &k).await;
    assert_eq!(again.status, StatusCode::CREATED);
    assert_eq!(first.body, again.body);
    let rounds: i64 = sqlx::query_scalar("SELECT count(*) FROM rounds")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rounds, 1);
    let res = send(&app, "GET", "/api/v1/casual/runs/active", None, Some(&ana)).await;
    assert_eq!(res.body["balance"], 650, "se pagó una sola vez");

    // Misma clave en una acción → misma respuesta, una sola carta.
    stack(&pool, &run, 6, &[Two, Six, Three, Ten, Four, Two]).await;
    let res = deal_round(&app, &ana, 100, &key()).await;
    let round = res.body["id"].as_str().unwrap().to_string();
    let k = key();
    let first = act(&app, &ana, &round, "hit", &k).await;
    let again = act(&app, &ana, &round, "hit", &k).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.body);
    assert_eq!(first.body, again.body);
    let cards = |v: &Value| v["player"]["hands"][0]["cards"].as_array().unwrap().len();
    assert_eq!(cards(&first.body), 3);
    let res = act(&app, &ana, &round, "hit", &key()).await;
    assert_eq!(cards(&res.body), 4);

    // Una clave que no es UUID → 400.
    let res = deal_round(&app, &ana, 10, "no-es-uuid").await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrations = "./migrations")]
async fn casual_bets_and_bust(pool: PgPool) {
    use Face::*;
    let app = test_app(pool.clone(), 10);
    let ana = register(&app, "ana@example.com").await;

    // Sin partida activa no se reparte.
    let res = deal_round(&app, &ana, 10, &key()).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(res.body["code"], "no_active_run");

    let run = new_run(&app, &ana, "vegas").await;
    for bet in [0, 5, 15, 510] {
        let res = deal_round(&app, &ana, bet, &key()).await;
        assert_eq!(res.status, StatusCode::BAD_REQUEST, "{bet}");
        assert_eq!(res.body["code"], "bet_out_of_range");
    }
    set_balance(&pool, &run, 100).await;
    let res = deal_round(&app, &ana, 200, &key()).await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["code"], "insufficient_funds");

    // Con todo el saldo apostado no alcanza para doblar ni dividir.
    stack(&pool, &run, 6, &[Eight, Six, Eight, Ten, Ten]).await;
    let res = deal_round(&app, &ana, 100, &key()).await;
    assert_eq!(res.body["wallet"]["maxBet"], 100);
    assert_eq!(res.body["allowed"]["split"], false);
    assert_eq!(res.body["allowed"]["double"], false);
    let round = res.body["id"].as_str().unwrap().to_string();
    let res = act(&app, &ana, &round, "double", &key()).await;
    assert_eq!(res.body["code"], "action_not_allowed");

    // Pide con 16, saca un 10 y se pasa: pierde todo el saldo.
    let res = act(&app, &ana, &round, "hit", &key()).await;
    assert_eq!(res.body["phase"], "settled");
    assert_eq!(res.body["player"]["hands"][0]["outcome"], "bust");
    assert_eq!(res.body["wallet"], json!({ "balance": 0, "maxBet": 0 }));

    // Con saldo menor que 10 la partida queda `busted`.
    let res = send(&app, "GET", "/api/v1/casual/runs/active", None, Some(&ana)).await;
    assert_eq!(res.body["code"], "no_active_run");
    let res = send(
        &app,
        "POST",
        "/api/v1/play/rounds",
        Some(json!({ "kind": "casual", "contextId": run, "bet": 10, "idempotencyKey": key() })),
        Some(&ana),
    )
    .await;
    assert_eq!(res.body["code"], "run_finished");
    let status: String = sqlx::query_scalar("SELECT status FROM casual_runs WHERE id = $1::uuid")
        .bind(&run)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "busted");
    // Y se puede empezar otra.
    new_run(&app, &ana, "europe").await;
}

/// El límite de las rutas de juego es por usuario, no por IP.
#[sqlx::test(migrations = "./migrations")]
async fn play_is_rate_limited_per_user(pool: PgPool) {
    let mut cfg = config(10);
    cfg.play_burst = 2;
    let app = app(AppState::new(pool, &cfg), &cfg, &RateLimits::new(&cfg));
    let ana = register(&app, "ana@example.com").await;
    let beto = register(&app, "beto@example.com").await;
    new_run(&app, &ana, "vegas").await;
    let res = deal_round(&app, &ana, 10, "no-es-uuid").await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    let res = deal_round(&app, &ana, 10, "no-es-uuid").await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(res.body["code"], "rate_limited");
    // La recarga es de 200 ms, pero nunca se pide esperar 0 s.
    assert_eq!(
        res.body["message"],
        "demasiadas peticiones; reintenta en 1 s"
    );
    // Beto, desde la misma IP, tiene su propio límite.
    new_run(&app, &beto, "vegas").await;
}

async fn set_run(pool: &PgPool, run: &str, sql_set: &str) {
    // Solo con literales de los tests.
    let sql = format!("UPDATE casual_runs SET {sql_set} WHERE id = $1::uuid");
    sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(run)
        .execute(pool)
        .await
        .unwrap();
}

#[sqlx::test(migrations = "./migrations")]
async fn casual_leaderboard_and_records(pool: PgPool) {
    let app = test_app(pool.clone(), 10);
    let cookie = |res: Res| res.cookie.unwrap();
    let ana = cookie(register_named(&app, "ana@example.com", "Ana").await);
    let beto = cookie(register_named(&app, "beto@example.com", "Beto").await);
    let carla = cookie(register_named(&app, "carla@example.com", "Carla").await);

    let res = send(&app, "GET", "/api/v1/me/records", None, Some(&ana)).await;
    assert_eq!(
        res.body,
        json!({ "casual": { "runs": 0 }, "ranked": { "seasons": [] }, "daily": { "finished": 0 } })
    );

    let a1 = new_run(&app, &ana, "vegas").await;
    set_run(&pool, &a1, "peak = 900, hands = 40, status = 'retired'").await;
    let a2 = new_run(&app, &ana, "vegas").await;
    set_run(&pool, &a2, "peak = 700, hands = 70").await;
    let b = new_run(&app, &beto, "vegas").await;
    set_run(&pool, &b, "peak = 1200").await;
    // Una partida de la semana pasada no cuenta.
    let c = new_run(&app, &carla, "vegas").await;
    set_run(
        &pool,
        &c,
        "peak = 5000, started_at = now() - interval '8 days'",
    )
    .await;

    let uri = "/api/v1/casual/leaderboard/weekly";
    let res = send(&app, "GET", uri, None, Some(&ana)).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(
        res.body["top"],
        json!([
            { "rank": 1, "displayName": "Beto", "value": 1200, "isMe": false },
            { "rank": 2, "displayName": "Ana", "value": 900, "isMe": true },
        ])
    );
    assert_eq!(res.body["me"]["rank"], 2);
    assert!(res.body["weekStart"]
        .as_str()
        .unwrap()
        .ends_with("T00:00:00Z"));

    // Sin sesión también se ve, sin `me`; sin partidas esta semana, tampoco.
    let res = send(&app, "GET", uri, None, None).await;
    assert_eq!(res.body["top"].as_array().unwrap().len(), 2);
    assert!(res.body.get("me").is_none());
    let res = send(&app, "GET", uri, None, Some(&carla)).await;
    assert!(res.body.get("me").is_none());

    let res = send(&app, "GET", "/api/v1/me/records", None, Some(&ana)).await;
    assert_eq!(
        res.body,
        json!({
            "casual": { "runs": 2, "bestPeak": 900, "mostHands": 70 },
            "ranked": { "seasons": [] },
            "daily": { "finished": 0 }
        })
    );
}

// ---- Ranked ----

async fn ranked_me(app: &Router, cookie: &str) -> Value {
    let res = send(app, "GET", "/api/v1/ranked/me", None, Some(cookie)).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    res.body
}

async fn ranked_deal(app: &Router, cookie: &str, bet: i64) -> Res {
    send(
        app,
        "POST",
        "/api/v1/play/rounds",
        Some(json!({ "kind": "ranked", "bet": bet, "idempotencyKey": key() })),
        Some(cookie),
    )
    .await
}

async fn set_account(pool: &PgPool, account: &Value, sql_set: &str) {
    // Solo con literales de los tests.
    let sql = format!("UPDATE ranked_accounts SET {sql_set} WHERE id = $1::uuid");
    sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(account["id"].as_str().unwrap())
        .execute(pool)
        .await
        .unwrap();
}

async fn stack_account(pool: &PgPool, account: &Value, top: &[Face]) {
    let shoe = serde_json::to_value(GameShoe::stacked(6, top)).unwrap();
    sqlx::query("UPDATE ranked_accounts SET shoe = $2 WHERE id = $1::uuid")
        .bind(account["id"].as_str().unwrap())
        .bind(shoe)
        .execute(pool)
        .await
        .unwrap();
}

/// Reparte 10,7 contra 10 + oculta 10 y se planta: pierde la apuesta.
async fn ranked_lose(
    app: &Router,
    pool: &PgPool,
    cookie: &str,
    account: &Value,
    bet: i64,
) -> Value {
    use Face::*;
    stack_account(pool, account, &[Ten, Ten, Seven, Ten]).await;
    let res = ranked_deal(app, cookie, bet).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let round = res.body["id"].as_str().unwrap().to_string();
    let res = act(app, cookie, &round, "stand", &key()).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["result"]["net"], -bet);
    res.body
}

#[sqlx::test(migrations = "./migrations")]
async fn ranked_account_and_bets(pool: PgPool) {
    let app = test_app(pool.clone(), 10);
    let ana = register(&app, "ana@example.com").await;
    let res = send(&app, "GET", "/api/v1/ranked/me", None, None).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);

    let acc = ranked_me(&app, &ana).await;
    let season = acc["season"].as_str().unwrap();
    assert_eq!(season.len(), 7, "AAAA-MM: {season}");
    assert_eq!(acc["balance"], 500);
    assert_eq!(acc["peak"], 500);
    assert_eq!(acc["hands"], 0);
    assert_eq!(acc["net"], 0);
    assert_eq!(acc["refilledTotal"], 0);
    assert_eq!(acc["maxBet"], 120, "25 % de 500, en decenas");
    assert_eq!(acc["minHands"], 200);
    assert!(acc.get("refillAvailableAt").is_none());
    // Una cuenta por temporada.
    assert_eq!(ranked_me(&app, &ana).await["id"], acc["id"]);

    for bet in [0, 5, 15, 130] {
        let res = ranked_deal(&app, &ana, bet).await;
        assert_eq!(res.status, StatusCode::BAD_REQUEST, "apuesta {bet}");
        assert_eq!(res.body["code"], "bet_out_of_range");
    }
    set_account(&pool, &acc, "balance = 1000").await;
    assert_eq!(ranked_me(&app, &ana).await["maxBet"], 250);
    set_account(&pool, &acc, "balance = 4000").await;
    assert_eq!(ranked_me(&app, &ana).await["maxBet"], 500);
    assert_eq!(ranked_me(&app, &ana).await["net"], 3500);

    // El nombre sale en el ranking: no se juega con el provisional.
    sqlx::query("UPDATE users SET display_name_set = false")
        .execute(&pool)
        .await
        .unwrap();
    let res = ranked_deal(&app, &ana, 10).await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["code"], "display_name_required");
}

#[sqlx::test(migrations = "./migrations")]
async fn ranked_round_has_no_aids_and_reviews_each_decision(pool: PgPool) {
    use Face::*;
    let app = test_app(pool.clone(), 10);
    let ana = register(&app, "ana@example.com").await;
    let acc = ranked_me(&app, &ana).await;

    // Jugador 10,5 contra 9 (+ oculta 7): pide un 3 (18) y se planta; el dealer
    // pide con 16 y se pasa con un 10.
    stack_account(&pool, &acc, &[Ten, Nine, Five, Seven, Three, Ten]).await;
    let res = ranked_deal(&app, &ana, 20).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let view = res.body;
    assert_eq!(view["kind"], "ranked");
    assert_eq!(view["contextId"], acc["id"]);
    assert!(view.get("shoeCounts").is_none(), "sin ayudas en ranked");
    assert_eq!(view["wallet"], json!({ "balance": 500, "maxBet": 120 }));
    let res = send(&app, "GET", "/api/v1/play/active", None, Some(&ana)).await;
    assert_eq!(res.body, view);
    let round = view["id"].as_str().unwrap().to_string();

    let res = act(&app, &ana, &round, "hit", &key()).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert!(res.body.get("shoeCounts").is_none());
    assert!(
        res.body.get("result").is_none(),
        "la revisión llega al terminar"
    );
    let res = act(&app, &ana, &round, "stand", &key()).await;
    assert_eq!(res.body["phase"], "settled");
    assert_eq!(res.body["result"]["net"], 20);
    assert_eq!(res.body["wallet"]["balance"], 520);
    let review = res.body["result"]["review"].as_array().unwrap().clone();
    assert_eq!(review.len(), 2);

    // La revisión coincide con /analyze/hand para lo que veía el jugador: sus
    // cartas, el 9 y la oculta contada como no vista.
    let mut shoe = [24u32; 10];
    shoe[9] = 96;
    for rank in [9, 8, 4] {
        shoe[rank] -= 1;
    }
    let analysis = |hand: Value, shoe: [u32; 10], allowed: Value| {
        json!({
            "version": 1, "rules": rules(), "shoe": shoe, "hand": hand, "dealerUp": 8,
            "splitHandsSoFar": 0, "isSplitAces": false, "allowed": allowed
        })
    };
    let first = analysis(
        json!([9, 4]),
        shoe,
        json!({ "double": true, "split": false, "surrender": true }),
    );
    shoe[2] -= 1;
    let second = analysis(
        json!([9, 4, 2]),
        shoe,
        json!({ "double": false, "split": false, "surrender": false }),
    );
    for (item, (req, chosen)) in review.iter().zip([(first, "hit"), (second, "stand")]) {
        let res = send(&app, "POST", "/api/v1/analyze/hand", Some(req), None).await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.body);
        let ev = |action: &Value| {
            res.body["actions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|a| &a["action"] == action)
                .unwrap()["ev"]
                .clone()
        };
        let best = &res.body["best"];
        assert_eq!(item["handIndex"], 0);
        assert_eq!(item["chosen"], chosen);
        assert_eq!(&item["best"], best);
        assert_eq!(item["evChosen"], ev(&json!(chosen)));
        assert_eq!(item["evBest"], ev(best));
    }

    let acc = ranked_me(&app, &ana).await;
    assert_eq!((&acc["hands"], &acc["peak"]), (&json!(1), &json!(520)));
}

#[sqlx::test(migrations = "./migrations")]
async fn ranked_refills_once_a_day(pool: PgPool) {
    let app = test_app(pool.clone(), 10);
    let ana = register(&app, "ana@example.com").await;
    let acc = ranked_me(&app, &ana).await;

    // Al bajar de 100 se recarga hasta 100 (la primera vez siempre se puede).
    set_account(&pool, &acc, "balance = 110").await;
    let view = ranked_lose(&app, &pool, &ana, &acc, 20).await;
    assert_eq!(view["result"]["refilled"], 10);
    assert_eq!(view["wallet"]["balance"], 100);
    assert!(view["wallet"].get("refillAvailableAt").is_none());

    // Antes de 24 h no hay otra recarga: se sigue con lo que queda.
    set_account(&pool, &acc, "balance = 30").await;
    let view = ranked_lose(&app, &pool, &ana, &acc, 10).await;
    assert!(view["result"].get("refilled").is_none());
    assert_eq!(view["wallet"]["balance"], 20);
    assert!(view["wallet"]["refillAvailableAt"].is_string());
    set_account(&pool, &acc, "balance = 5").await;
    let res = ranked_deal(&app, &ana, 10).await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["code"], "refill_pending");
    let me = ranked_me(&app, &ana).await;
    assert_eq!(me["maxBet"], 0);
    assert!(me["refillAvailableAt"].is_string());

    // Pasadas 24 h, la recarga se aplica al volver; no suma a la ganancia neta.
    set_account(&pool, &acc, "last_refill_at = now() - interval '25 hours'").await;
    let me = ranked_me(&app, &ana).await;
    assert_eq!(me["balance"], 100);
    assert_eq!(me["refilledTotal"], 105);
    assert_eq!(me["net"], 100 - 500 - 105);
    assert!(me.get("refillAvailableAt").is_none());
    let refills: i64 = sqlx::query_scalar("SELECT count(*) FROM ranked_refills")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(refills, 2);
}

#[sqlx::test(migrations = "./migrations")]
async fn ranked_leaderboard_and_seasons(pool: PgPool) {
    let app = test_app(pool.clone(), 10);
    let ana = register(&app, "ana@example.com").await;
    let beto = register(&app, "beto@example.com").await;
    let carla = register(&app, "carla@example.com").await;
    let a = ranked_me(&app, &ana).await;
    let b = ranked_me(&app, &beto).await;
    let c = ranked_me(&app, &carla).await;
    set_account(&pool, &a, "hands = 200, balance = 900").await;
    set_account(&pool, &b, "hands = 250, balance = 600, refilled_total = 50").await;
    // Más ganancia, pero sin el mínimo de manos.
    set_account(&pool, &c, "hands = 199, balance = 2000").await;

    let uri = "/api/v1/ranked/leaderboard";
    let res = send(&app, "GET", uri, None, Some(&carla)).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["season"], a["season"]);
    assert_eq!(res.body["closed"], false);
    assert_eq!(
        res.body["top"],
        json!([
            { "rank": 1, "displayName": "ana", "net": 400, "hands": 200,
              "provisional": false, "isMe": false },
            { "rank": 2, "displayName": "beto", "net": 50, "hands": 250,
              "provisional": false, "isMe": false }
        ])
    );
    assert_eq!(
        res.body["me"],
        json!({ "rank": 1, "displayName": "carla", "net": 1500, "hands": 199,
                "provisional": true, "isMe": true })
    );
    let res = send(&app, "GET", uri, None, Some(&beto)).await;
    assert_eq!(res.body["top"][1]["isMe"], true);
    assert_eq!(res.body["me"]["rank"], 2);
    let res = send(&app, "GET", uri, None, None).await;
    assert!(res.body.get("me").is_none());
    let res = send(&app, "GET", &format!("{uri}?season=1999-01"), None, None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);

    // Una temporada terminada se cierra con posiciones y medallas, pero no mientras
    // tenga una ronda en juego.
    sqlx::query(
        "INSERT INTO seasons (id, starts_at, ends_at)
         VALUES ('2020-01', '2020-01-01T00:00:00Z', '2020-02-01T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE ranked_accounts SET season_id = '2020-01'")
        .execute(&pool)
        .await
        .unwrap();
    let res = ranked_deal(&app, &beto, 10).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    // La ronda nueva va a la cuenta de la temporada actual.
    assert_ne!(res.body["contextId"], b["id"]);
    let round = res.body["id"].as_str().unwrap().to_string();
    // Simula una ronda que empezó en la temporada vieja y sigue en juego.
    sqlx::query("UPDATE rounds SET context_id = $1::uuid")
        .bind(b["id"].as_str().unwrap())
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(api::routes::ranked::close_seasons(&pool).await.unwrap(), 0);
    sqlx::query("UPDATE rounds SET status = 'settled' WHERE id = $1::uuid")
        .bind(&round)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(api::routes::ranked::close_seasons(&pool).await.unwrap(), 1);
    assert_eq!(api::routes::ranked::close_seasons(&pool).await.unwrap(), 0);

    let res = send(
        &app,
        "GET",
        &format!("{uri}?season=2020-01"),
        None,
        Some(&carla),
    )
    .await;
    assert_eq!(res.body["closed"], true);
    assert_eq!(res.body["top"][0]["medal"], "gold");
    assert_eq!(res.body["top"][1]["medal"], "silver");
    assert!(res.body["me"].get("medal").is_none());

    let res = send(&app, "GET", "/api/v1/me/records", None, Some(&ana)).await;
    assert_eq!(
        res.body["ranked"],
        json!({
            "seasons": [{ "season": "2020-01", "rank": 1, "net": 400, "hands": 200,
                          "medal": "gold" }],
            "bestRank": 1
        })
    );
    let res = send(&app, "GET", "/api/v1/me/records", None, Some(&carla)).await;
    assert_eq!(res.body["ranked"], json!({ "seasons": [] }));

    // Logro de medalla al cerrar la temporada, solo para quien la recibió.
    let res = send(&app, "GET", "/api/v1/me/achievements", None, Some(&ana)).await;
    assert_eq!(achievement_ids(&res.body), ["rankedMedal"]);
    let res = send(&app, "GET", "/api/v1/me/achievements", None, Some(&carla)).await;
    assert!(achievement_ids(&res.body).is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn stale_rounds_are_closed_by_standing(pool: PgPool) {
    use Face::*;
    let app = test_app(pool.clone(), 10);
    let ana = register(&app, "ana@example.com").await;
    let run = new_run(&app, &ana, "vegas").await;
    // Jugador 10,8 contra 6 + oculta 10: el dealer pide un 5 → 21.
    stack(&pool, &run, 6, &[Ten, Six, Eight, Ten, Five]).await;
    let res = deal_round(&app, &ana, 50, &key()).await;
    let round = res.body["id"].as_str().unwrap().to_string();

    // Con actividad reciente no se toca.
    assert_eq!(
        api::routes::play::close_stale_rounds(&pool).await.unwrap(),
        0
    );
    sqlx::query("UPDATE rounds SET created_at = now() - interval '25 hours'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        api::routes::play::close_stale_rounds(&pool).await.unwrap(),
        1
    );

    let res = send(&app, "GET", "/api/v1/play/active", None, Some(&ana)).await;
    assert_eq!(res.body["code"], "no_active_round");
    let res = send(&app, "GET", "/api/v1/casual/runs/active", None, Some(&ana)).await;
    assert_eq!(res.body["balance"], 450);
    assert_eq!(res.body["hands"], 1);
    let res = act(&app, &ana, &round, "hit", &key()).await;
    assert_eq!(res.body["code"], "no_active_round");
}

// ---- Reto diario ----

async fn daily_start(app: &Router, cookie: &str) -> Res {
    send(app, "POST", "/api/v1/daily/attempts", None, Some(cookie)).await
}

async fn daily_deal(app: &Router, cookie: &str, bet: i64) -> Res {
    send(
        app,
        "POST",
        "/api/v1/play/rounds",
        Some(json!({ "kind": "daily", "bet": bet, "idempotencyKey": key() })),
        Some(cookie),
    )
    .await
}

async fn set_attempt(pool: &PgPool, attempt: &Value, sql_set: &str) {
    // Solo con literales de los tests.
    let sql = format!("UPDATE daily_attempts SET {sql_set} WHERE id = $1::uuid");
    sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(attempt["id"].as_str().unwrap())
        .execute(pool)
        .await
        .unwrap();
}

#[sqlx::test(migrations = "./migrations")]
async fn daily_attempt_once_a_day(pool: PgPool) {
    let app = test_app(pool.clone(), 10);
    let ana = register(&app, "ana@example.com").await;

    let res = send(&app, "GET", "/api/v1/daily/me", None, Some(&ana)).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(res.body["code"], "no_daily_attempt");
    let res = daily_deal(&app, &ana, 10).await;
    assert_eq!(res.body["code"], "no_daily_attempt");

    let res = daily_start(&app, &ana).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let attempt = res.body;
    assert_eq!(attempt["day"].as_str().unwrap().len(), 10, "AAAA-MM-DD");
    assert_eq!(attempt["status"], "active");
    assert_eq!(attempt["balance"], 500);
    assert_eq!(attempt["hands"], 0);
    assert_eq!(attempt["handsTotal"], 30);
    assert_eq!(attempt["maxBet"], 500);
    let res = daily_start(&app, &ana).await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["code"], "daily_taken");
    let res = send(&app, "GET", "/api/v1/daily/me", None, Some(&ana)).await;
    assert_eq!(res.body, attempt);

    // El nombre sale en el ranking diario.
    let beto = register(&app, "beto@example.com").await;
    sqlx::query("UPDATE users SET display_name_set = false WHERE email = 'beto@example.com'")
        .execute(&pool)
        .await
        .unwrap();
    let res = daily_start(&app, &beto).await;
    assert_eq!(res.body["code"], "display_name_required");
}

#[sqlx::test(migrations = "./migrations")]
async fn daily_shoe_is_the_same_for_everyone(pool: PgPool) {
    let app = test_app(pool.clone(), 10);
    let ana = register(&app, "ana@example.com").await;
    let beto = register(&app, "beto@example.com").await;
    daily_start(&app, &ana).await;
    daily_start(&app, &beto).await;

    let a = daily_deal(&app, &ana, 10).await;
    let b = daily_deal(&app, &beto, 50).await;
    assert_eq!(a.status, StatusCode::CREATED, "{}", a.body);
    assert_eq!(a.body["kind"], "daily");
    assert_eq!(
        a.body["player"]["hands"][0]["cards"],
        b.body["player"]["hands"][0]["cards"]
    );
    assert_eq!(a.body["dealer"]["up"], b.body["dealer"]["up"]);
    assert!(a.body.get("shoeCounts").is_none(), "sin ayudas en el reto");
}

#[sqlx::test(migrations = "./migrations")]
async fn daily_attempt_ends_after_thirty_hands(pool: PgPool) {
    use Face::*;
    let app = test_app(pool.clone(), 10);
    let ana = register(&app, "ana@example.com").await;
    let attempt = daily_start(&app, &ana).await.body;
    set_attempt(&pool, &attempt, "hands = 29").await;
    // Jugador 10,7 contra 10 + oculta 10: se planta y pierde.
    let shoe = serde_json::to_value(GameShoe::stacked(6, &[Ten, Ten, Seven, Ten])).unwrap();
    sqlx::query("UPDATE daily_attempts SET shoe = $1")
        .bind(shoe)
        .execute(&pool)
        .await
        .unwrap();

    let res = daily_deal(&app, &ana, 100).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let round = res.body["id"].as_str().unwrap().to_string();
    let res = act(&app, &ana, &round, "stand", &key()).await;
    assert_eq!(res.body["phase"], "settled");
    assert_eq!(res.body["result"]["net"], -100);
    assert_eq!(res.body["result"]["review"][0]["chosen"], "stand");
    assert_eq!(res.body["wallet"], json!({ "balance": 400, "maxBet": 0 }));

    let res = send(&app, "GET", "/api/v1/daily/me", None, Some(&ana)).await;
    assert_eq!(res.body["status"], "finished");
    assert_eq!(res.body["hands"], 30);
    assert!(res.body["finishedAt"].is_string());
    let res = daily_deal(&app, &ana, 10).await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["code"], "daily_finished");
}

#[sqlx::test(migrations = "./migrations")]
async fn daily_leaderboard_and_records(pool: PgPool) {
    let app = test_app(pool.clone(), 10);
    let ana = register(&app, "ana@example.com").await;
    let beto = register(&app, "beto@example.com").await;
    let carla = register(&app, "carla@example.com").await;
    let a = daily_start(&app, &ana).await.body;
    let b = daily_start(&app, &beto).await.body;
    let c = daily_start(&app, &carla).await.body;
    set_attempt(&pool, &a, "balance = 900, hands = 30, status = 'finished'").await;
    set_attempt(&pool, &b, "balance = 700, hands = 30, status = 'finished'").await;
    // Más saldo, pero sin terminar: no entra en el top.
    set_attempt(&pool, &c, "balance = 2000, hands = 10").await;

    let uri = "/api/v1/daily/leaderboard";
    let res = send(&app, "GET", uri, None, Some(&carla)).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["day"], a["day"]);
    assert_eq!(
        res.body["top"],
        json!([
            { "rank": 1, "displayName": "ana", "balance": 900, "hands": 30,
              "finished": true, "isMe": false },
            { "rank": 2, "displayName": "beto", "balance": 700, "hands": 30,
              "finished": true, "isMe": false }
        ])
    );
    assert_eq!(
        res.body["me"],
        json!({ "rank": 1, "displayName": "carla", "balance": 2000, "hands": 10,
                "finished": false, "isMe": true })
    );
    let res = send(&app, "GET", uri, None, None).await;
    assert!(res.body.get("me").is_none());
    let res = send(&app, "GET", &format!("{uri}?day=ayer"), None, None).await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    let res = send(&app, "GET", &format!("{uri}?day=2020-01-01"), None, None).await;
    assert_eq!(res.body["top"], json!([]));

    // La mejor posición solo cuenta días ya terminados.
    let res = send(&app, "GET", "/api/v1/me/records", None, Some(&ana)).await;
    assert_eq!(
        res.body["daily"],
        json!({ "finished": 1, "bestBalance": 900 })
    );
    sqlx::query("UPDATE daily_attempts SET day = day - 1")
        .execute(&pool)
        .await
        .unwrap();
    let res = send(&app, "GET", "/api/v1/me/records", None, Some(&beto)).await;
    assert_eq!(
        res.body["daily"],
        json!({ "finished": 1, "bestBalance": 700, "bestRank": 2 })
    );
}

// ---- Logros ----

fn achievement_ids(body: &Value) -> Vec<String> {
    let mut ids: Vec<String> = body["unlocked"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["id"].as_str().unwrap().to_string())
        .collect();
    ids.sort();
    ids
}

#[sqlx::test(migrations = "./migrations")]
async fn achievements_unlock_once(pool: PgPool) {
    use Face::*;
    let app = test_app(pool.clone(), 10);
    let ana = register(&app, "ana@example.com").await;
    let run = new_run(&app, &ana, "vegas").await;
    let res = send(&app, "GET", "/api/v1/me/achievements", None, Some(&ana)).await;
    assert_eq!(
        res.body,
        json!({ "unlocked": [], "optimalStreak": 0, "bestOptimalStreak": 0 })
    );

    // Blackjack contra un 9: se liquida en el reparto.
    stack(&pool, &run, 6, &[Ace, Nine, K, Seven]).await;
    let k = key();
    let res = deal_round(&app, &ana, 10, &k).await;
    assert_eq!(res.body["phase"], "settled", "{}", res.body);
    assert_eq!(
        res.body["result"]["unlocked"],
        json!(["firstWin", "blackjack"])
    );
    // La respuesta repetida trae lo mismo.
    assert_eq!(deal_round(&app, &ana, 10, &k).await.body, res.body);

    // Ya los tiene: no se vuelven a anunciar.
    stack(&pool, &run, 6, &[Ace, Nine, K, Seven]).await;
    let res = deal_round(&app, &ana, 10, &key()).await;
    assert!(res.body["result"].get("unlocked").is_none());

    let res = send(&app, "GET", "/api/v1/me/achievements", None, Some(&ana)).await;
    assert_eq!(achievement_ids(&res.body), ["blackjack", "firstWin"]);
    assert!(res.body["unlocked"][0]["unlockedAt"].is_string());
    // Casual no tiene revisión: no cuenta para la racha.
    assert_eq!(res.body["optimalStreak"], 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn optimal_streak_in_ranked(pool: PgPool) {
    use Face::*;
    let app = test_app(pool.clone(), 10);
    let ana = register(&app, "ana@example.com").await;
    let acc = ranked_me(&app, &ana).await;
    sqlx::query(
        "INSERT INTO player_stats (user_id, optimal_streak, best_optimal_streak)
         SELECT id, 9, 9 FROM users",
    )
    .execute(&pool)
    .await
    .unwrap();

    // 20 contra 9 (+ oculta 7): plantarse es lo óptimo y llega a 10 seguidas. El dealer
    // pide con 16 y se pasa.
    stack_account(&pool, &acc, &[Ten, Nine, K, Seven, Ten]).await;
    let round = ranked_deal(&app, &ana, 10).await.body["id"]
        .as_str()
        .unwrap()
        .to_string();
    let res = act(&app, &ana, &round, "stand", &key()).await;
    assert_eq!(res.body["result"]["review"][0]["best"], "stand");
    assert_eq!(
        res.body["result"]["unlocked"],
        json!(["firstWin", "optimal10"])
    );
    let res = send(&app, "GET", "/api/v1/me/achievements", None, Some(&ana)).await;
    assert_eq!(
        (&res.body["optimalStreak"], &res.body["bestOptimalStreak"]),
        (&json!(10), &json!(10))
    );

    // Pedir con 20 no es óptimo: la racha vuelve a 0 y la mejor se conserva.
    stack_account(&pool, &acc, &[Ten, Nine, K, Seven, Ten]).await;
    let round = ranked_deal(&app, &ana, 10).await.body["id"]
        .as_str()
        .unwrap()
        .to_string();
    let res = act(&app, &ana, &round, "hit", &key()).await;
    assert_eq!(res.body["phase"], "settled", "se pasa con 30");
    assert!(res.body["result"].get("unlocked").is_none());
    let res = send(&app, "GET", "/api/v1/me/achievements", None, Some(&ana)).await;
    assert_eq!(
        (&res.body["optimalStreak"], &res.body["bestOptimalStreak"]),
        (&json!(0), &json!(10))
    );
}

// ---- Cosméticos ----

#[sqlx::test(migrations = "./migrations")]
async fn cosmetics_follow_achievements(pool: PgPool) {
    let app = test_app(pool.clone(), 10);
    let res = send(&app, "GET", "/api/v1/cosmetics", None, None).await;
    assert_eq!(res.status, StatusCode::OK);
    let catalog = res.body.as_array().unwrap();
    assert_eq!(catalog.len(), 14);
    assert_eq!(catalog[0], json!({ "id": "feltGreen", "slot": "felt" }));
    assert!(catalog
        .iter()
        .any(|c| c == &json!({ "id": "backGold", "slot": "cardBack", "unlockedBy": "blackjack" })));

    let ana = register(&app, "ana@example.com").await;
    let uri = "/api/v1/me/cosmetics";
    let defaults = json!({
        "felt": "feltGreen", "cardBack": "backClassic", "chips": "chipsClassic", "title": null
    });
    let res = send(&app, "GET", uri, None, Some(&ana)).await;
    assert_eq!(
        res.body,
        json!({ "owned": ["feltGreen", "backClassic", "chipsClassic"], "equipped": defaults })
    );

    // Bloqueado hasta tener el logro; cada uno en su lugar.
    let gold = json!({
        "felt": "feltGreen", "cardBack": "backGold", "chips": "chipsClassic", "title": "titleRookie"
    });
    let res = send(&app, "PUT", uri, Some(gold.clone()), Some(&ana)).await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["code"], "cosmetic_locked");
    let wrong = json!({
        "felt": "backClassic", "cardBack": "backClassic", "chips": "chipsClassic", "title": null
    });
    let res = send(&app, "PUT", uri, Some(wrong), Some(&ana)).await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    assert_eq!(res.body["code"], "cosmetic_wrong_slot");

    sqlx::query(
        "INSERT INTO user_achievements (user_id, achievement)
         SELECT id, unnest(ARRAY['blackjack', 'firstWin']) FROM users",
    )
    .execute(&pool)
    .await
    .unwrap();
    let res = send(&app, "PUT", uri, Some(gold.clone()), Some(&ana)).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["equipped"], gold);
    assert_eq!(
        res.body["owned"],
        json!([
            "feltGreen",
            "backClassic",
            "backGold",
            "chipsClassic",
            "titleRookie"
        ])
    );
    let res = send(&app, "GET", uri, None, Some(&ana)).await;
    assert_eq!(res.body["equipped"], gold);

    // Quitar el título y volver a los de serie.
    let res = send(&app, "PUT", uri, Some(defaults.clone()), Some(&ana)).await;
    assert_eq!(res.body["equipped"], defaults);
    let res = send(&app, "GET", uri, None, None).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = "./migrations")]
async fn titles_show_in_leaderboards(pool: PgPool) {
    let app = test_app(pool.clone(), 10);
    let ana = register(&app, "ana@example.com").await;
    let beto = register(&app, "beto@example.com").await;
    sqlx::query(
        "INSERT INTO user_achievements (user_id, achievement)
         SELECT id, 'firstWin' FROM users WHERE email = 'ana@example.com'",
    )
    .execute(&pool)
    .await
    .unwrap();
    let equipped = json!({
        "felt": "feltGreen", "cardBack": "backClassic", "chips": "chipsClassic",
        "title": "titleRookie"
    });
    let res = send(
        &app,
        "PUT",
        "/api/v1/me/cosmetics",
        Some(equipped),
        Some(&ana),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);

    // Una fila en cada ranking para los dos.
    for cookie in [&ana, &beto] {
        let run = new_run(&app, cookie, "vegas").await;
        set_run(&pool, &run, "peak = 600").await;
        let acc = ranked_me(&app, cookie).await;
        set_account(&pool, &acc, "hands = 200").await;
        let attempt = daily_start(&app, cookie).await.body;
        set_attempt(&pool, &attempt, "hands = 30, status = 'finished'").await;
    }
    for uri in [
        "/api/v1/casual/leaderboard/weekly",
        "/api/v1/ranked/leaderboard",
        "/api/v1/daily/leaderboard",
    ] {
        let res = send(&app, "GET", uri, None, Some(&ana)).await;
        let top = res.body["top"].as_array().unwrap();
        assert_eq!(top.len(), 2, "{uri}");
        let row = |name: &str| {
            top.iter()
                .find(|r| r["displayName"] == name)
                .unwrap()
                .clone()
        };
        assert_eq!(row("ana")["title"], "titleRookie", "{uri}");
        assert!(row("beto").get("title").is_none(), "{uri}: sin título");
        assert_eq!(res.body["me"]["title"], "titleRookie", "{uri}");
    }
}

// ---- Seguro ----

async fn insure(app: &Router, cookie: &str, round: &str, take: bool, key: &str) -> Res {
    send(
        app,
        "POST",
        &format!("/api/v1/play/rounds/{round}/insurance"),
        Some(json!({ "take": take, "idempotencyKey": key })),
        Some(cookie),
    )
    .await
}

#[sqlx::test(migrations = "./migrations")]
async fn insurance_in_casual(pool: PgPool) {
    use Face::*;
    let app = test_app(pool.clone(), 10);
    let ana = register(&app, "ana@example.com").await;
    let run = new_run(&app, &ana, "vegas").await;

    // 10,7 contra As + K: se ofrece el seguro antes del peek.
    stack(&pool, &run, 6, &[Ten, Ace, Seven, K]).await;
    let res = deal_round(&app, &ana, 20, &key()).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let view = res.body;
    let round = view["id"].as_str().unwrap().to_string();
    assert_eq!(view["phase"], "insurance");
    assert_eq!(
        view["dealer"]["hole"],
        Value::Null,
        "el peek todavía no ocurrió"
    );
    assert_eq!(view["allowed"]["insurance"], true);
    assert_eq!(view["allowed"]["hit"], false);
    assert!(view.get("insured").is_none());
    let res = act(&app, &ana, &round, "hit", &key()).await;
    assert_eq!(res.body["code"], "action_not_allowed");

    // Con seguro: la mano pierde 20 y el seguro (10) gana 20.
    let k = key();
    let res = insure(&app, &ana, &round, true, &k).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["phase"], "settled");
    assert_eq!(res.body["peekBlackjack"], true);
    assert_eq!(res.body["insured"], true);
    assert_eq!(res.body["result"]["insuranceNet"], 20);
    assert_eq!(res.body["result"]["net"], 0);
    assert_eq!(res.body["wallet"]["balance"], 500);
    // Misma clave, misma respuesta; la ronda ya terminó para una clave nueva.
    assert_eq!(insure(&app, &ana, &round, true, &k).await.body, res.body);
    let res = insure(&app, &ana, &round, false, &key()).await;
    assert_eq!(res.body["code"], "no_active_round");

    // 10,9 contra As + 7: sin seguro se juega normal y gana a 18.
    stack(&pool, &run, 6, &[Ten, Ace, Nine, Seven]).await;
    let round = deal_round(&app, &ana, 20, &key()).await.body["id"]
        .as_str()
        .unwrap()
        .to_string();
    let res = insure(&app, &ana, &round, false, &key()).await;
    assert_eq!(res.body["phase"], "player");
    assert_eq!(res.body["insured"], false);
    let res = act(&app, &ana, &round, "stand", &key()).await;
    assert_eq!(res.body["result"]["net"], 20);
    assert!(res.body["result"].get("insuranceNet").is_none());

    // Sin fichas para la media apuesta no se puede asegurar.
    set_balance(&pool, &run, 20).await;
    stack(&pool, &run, 6, &[Ten, Ace, Nine, Seven]).await;
    let res = deal_round(&app, &ana, 20, &key()).await;
    assert_eq!(res.body["allowed"]["insurance"], false);
    let round = res.body["id"].as_str().unwrap().to_string();
    let res = insure(&app, &ana, &round, true, &key()).await;
    assert_eq!(res.body["code"], "action_not_allowed");
    assert_eq!(
        insure(&app, &ana, &round, false, &key()).await.body["phase"],
        "player"
    );
}
