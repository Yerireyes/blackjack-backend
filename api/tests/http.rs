//! Tests de punta a punta de la API contra PostgreSQL. Cada test recibe una base
//! nueva con las migraciones aplicadas (`#[sqlx::test]`).

use std::net::SocketAddr;

use api::auth::JwtKeys;
use api::config::Config;
use api::state::AppState;
use api::{app, RateLimits};
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;

fn config(analyze_burst: u32) -> Config {
    Config {
        database_url: String::new(),
        jwt_secret: b"secreto-de-prueba-con-al-menos-32-bytes".to_vec(),
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        frontend_origin: "http://localhost:5173".into(),
        cookie_secure: false,
        analyze_replenish_ms: 60_000,
        analyze_burst,
    }
}

fn test_app(pool: PgPool, analyze_burst: u32) -> Router {
    let cfg = config(analyze_burst);
    let state = AppState {
        pool,
        jwt: JwtKeys::new(&cfg.jwt_secret),
        cookie_secure: false,
    };
    app(state, &cfg, &RateLimits::new(&cfg))
}

struct Res {
    status: StatusCode,
    cookie: Option<String>,
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
    // Lo que pone axum::serve con into_make_service_with_connect_info (límite por IP).
    let mut req = req;
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40000))));
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    // "bj_session=<token>" de Set-Cookie, listo para mandarlo en Cookie.
    let cookie = res
        .headers()
        .get(header::SET_COOKIE)
        .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_string());
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    Res {
        status,
        cookie,
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

async fn register(app: &Router, email: &str) -> String {
    let res = send(
        app,
        "POST",
        "/api/v1/auth/register",
        Some(json!({ "email": email, "password": "contraseña-segura" })),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    res.cookie.expect("cookie de sesión")
}

#[sqlx::test(migrations = "./migrations")]
async fn health(pool: PgPool) {
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

#[sqlx::test(migrations = "./migrations")]
async fn auth_flow(pool: PgPool) {
    let app = test_app(pool, 10);
    let cookie = register(&app, "Ana@Example.com").await;
    assert!(cookie.starts_with("bj_session="));

    // Mismo email con otras mayúsculas → 409.
    let res = send(
        &app,
        "POST",
        "/api/v1/auth/register",
        Some(json!({ "email": "ana@example.com", "password": "otra-contraseña" })),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["code"], "email_taken");

    // Validaciones.
    for body in [
        json!({ "email": "no-es-email", "password": "contraseña-segura" }),
        json!({ "email": "b@example.com", "password": "corta" }),
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
