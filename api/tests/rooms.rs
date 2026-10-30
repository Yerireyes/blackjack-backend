//! Tests de las salas multijugador: el servidor escucha en un puerto real y los
//! jugadores se conectan con clientes WebSocket de verdad. El zapato viene armado para
//! saber qué cartas salen; los tiempos son cortos para no esperar.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use api::config::Config;
use api::rooms::{Rooms, RoomsConfig};
use api::state::AppState;
use api::{app, RateLimits};
use engine::game::{Face, GameShoe};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use sqlx::PgPool;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

fn config() -> Config {
    Config {
        database_url: String::new(),
        jwt_secret: b"secreto-de-prueba-con-al-menos-32-bytes".to_vec(),
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        frontend_origin: "http://localhost:5180".into(),
        cookie_secure: false,
        analyze_replenish_ms: 60_000,
        analyze_burst: 100,
        auth_burst: 1000,
        play_burst: 1000,
        max_rooms: 200,
        trusted_proxies: vec!["127.0.0.1".parse().unwrap()],
        google: None,
        daily_seed_secret: None,
    }
}

/// Tiempos cortos: las apuestas y los turnos se resuelven con mensajes, no esperando.
fn fast() -> RoomsConfig {
    RoomsConfig {
        bet_time: Duration::from_secs(5),
        turn_time: Duration::from_secs(5),
        results_time: Duration::from_millis(150),
        reconnect: Duration::from_millis(500),
        ..RoomsConfig::new(200)
    }
}

struct Server {
    base: String,
    http: reqwest::Client,
}

/// Levanta la API en un puerto libre; las salas reparten de un zapato con `top` arriba.
async fn server(pool: PgPool, rooms: RoomsConfig, top: &[Face]) -> Server {
    let cfg = config();
    let mut state = AppState::new(pool, &cfg);
    let top: Arc<Vec<Face>> = Arc::new(top.to_vec());
    state.rooms = Arc::new(Rooms::with_shoe(
        rooms,
        Arc::new(move |decks| GameShoe::stacked(decks, &top)),
    ));
    let router = app(state, &cfg, &RateLimits::new(&cfg));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    Server {
        base: format!("{addr}/api/v1"),
        http: reqwest::Client::new(),
    }
}

impl Server {
    /// Registra un usuario con nombre y devuelve la cookie de sesión.
    async fn register(&self, name: &str) -> String {
        let res = self
            .http
            .post(format!("http://{}/auth/register", self.base))
            .json(&json!({
                "email": format!("{name}@example.com"),
                "password": "contraseña-segura",
                "displayName": name
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 201);
        res.headers()
            .get_all("set-cookie")
            .iter()
            .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_string())
            .find(|c| c.starts_with("bj_session="))
            .unwrap()
    }

    async fn request(
        &self,
        method: &str,
        path: &str,
        cookie: Option<&str>,
        body: Option<Value>,
    ) -> (u16, Value) {
        let url = format!("http://{}{path}", self.base);
        let mut req = match method {
            "POST" => self.http.post(url),
            _ => self.http.get(url),
        };
        if let Some(c) = cookie {
            req = req.header("cookie", c);
        }
        if let Some(b) = body {
            req = req.json(&b);
        }
        let res = req.send().await.unwrap();
        let status = res.status().as_u16();
        (status, res.json().await.unwrap_or(Value::Null))
    }

    async fn create(&self, cookie: &str, body: Value) -> (u16, Value) {
        self.request("POST", "/rooms", Some(cookie), Some(body))
            .await
    }

    /// Crea una sala pública de 7 asientos con Las Vegas y devuelve el código.
    async fn room(&self, cookie: &str, aids: bool) -> String {
        let (status, body) = self
            .create(
                cookie,
                json!({ "name": "Mesa", "visibility": "public", "seats": 7,
                        "preset": "vegas", "aids": aids }),
            )
            .await;
        assert_eq!(status, 201, "{body}");
        body["code"].as_str().unwrap().to_string()
    }

    async fn try_connect(&self, code: &str, cookie: &str) -> Result<Ws, WsError> {
        let mut req = format!("ws://{}/rooms/{code}/ws", self.base)
            .into_client_request()
            .unwrap();
        req.headers_mut().insert("cookie", cookie.parse().unwrap());
        tokio_tungstenite::connect_async(req)
            .await
            .map(|(ws, _)| ws)
    }

    /// Se conecta y devuelve el `playerId` del `welcome`.
    async fn connect(&self, code: &str, cookie: &str) -> (Ws, String) {
        let mut ws = self.try_connect(code, cookie).await.unwrap();
        let welcome = recv(&mut ws).await;
        assert_eq!(welcome["type"], "welcome");
        let id = welcome["playerId"].as_str().unwrap().to_string();
        (ws, id)
    }
}

/// El siguiente mensaje JSON del servidor.
async fn recv(ws: &mut Ws) -> Value {
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("el servidor no respondió a tiempo")
            .expect("conexión cerrada")
            .unwrap();
        if let Message::Text(t) = msg {
            return serde_json::from_str(&t).unwrap();
        }
    }
}

/// Lee hasta el primer mensaje que cumpla `pred`.
async fn until(ws: &mut Ws, pred: impl Fn(&Value) -> bool) -> Value {
    loop {
        let msg = recv(ws).await;
        if pred(&msg) {
            return msg;
        }
    }
}

/// Lee estados hasta que la sala esté en `phase`.
async fn phase(ws: &mut Ws, phase: &str) -> Value {
    until(ws, |m| m["type"] == "state" && m["room"]["phase"] == phase).await["room"].clone()
}

async fn send(ws: &mut Ws, msg: Value) {
    ws.send(Message::Text(msg.to_string().into()))
        .await
        .unwrap();
}

async fn error(ws: &mut Ws) -> String {
    until(ws, |m| m["type"] == "error").await["code"]
        .as_str()
        .unwrap()
        .to_string()
}

#[sqlx::test(migrations = "./migrations")]
async fn rooms_http(pool: PgPool) {
    let s = server(
        pool.clone(),
        RoomsConfig {
            max_rooms: 2,
            ..fast()
        },
        &[],
    )
    .await;
    let ana = s.register("ana").await;
    let beto = s.register("beto").await;
    let carla = s.register("carla").await;

    let bad = [
        json!({ "name": "ab", "visibility": "public", "seats": 7, "preset": "vegas", "aids": false }),
        json!({ "name": "Mesa", "visibility": "public", "seats": 8, "preset": "vegas", "aids": false }),
        json!({ "name": "Mesa", "visibility": "public", "seats": 4, "preset": "vegas", "aids": false }),
        json!({ "name": "Mesa", "visibility": "public", "seats": 5, "aids": false }),
    ];
    for body in bad {
        let (status, res) = s.create(&ana, body.clone()).await;
        assert_eq!(status, 400, "{body} → {res}");
    }

    let (status, a) = s
        .create(
            &ana,
            json!({ "name": "  Mesa de Ana  ", "visibility": "public", "seats": 6,
                    "preset": "europe", "aids": true }),
        )
        .await;
    assert_eq!(status, 201, "{a}");
    let code = a["code"].as_str().unwrap();
    assert_eq!(code.len(), 6);
    assert_eq!(a["name"], "Mesa de Ana");
    assert_eq!(a["host"], "ana");
    assert_eq!(
        (&a["seats"], &a["seated"], &a["maxSpectators"]),
        (&json!(6), &json!(0), &json!(10))
    );
    assert_eq!(a["preset"], "europe");
    assert_eq!(a["rules"]["maxSplitHands"], 2);

    let (status, res) = s
        .create(
            &ana,
            json!({ "name": "Otra", "visibility": "public", "seats": 7,
                                               "preset": "vegas", "aids": false }),
        )
        .await;
    assert_eq!((status, &res["code"]), (409, &json!("room_hosting")));

    let (status, b) = s
        .create(
            &beto,
            json!({ "name": "Privada", "visibility": "private", "seats": 5,
                               "preset": "vegas", "aids": false }),
        )
        .await;
    assert_eq!(status, 201);
    let (status, res) = s
        .create(
            &carla,
            json!({ "name": "Tercera", "visibility": "public",
                                                 "seats": 7, "preset": "vegas", "aids": false }),
        )
        .await;
    assert_eq!((status, &res["code"]), (503, &json!("rooms_full")));

    // El lobby solo muestra las públicas; con el código se ve también la privada.
    let (_, list) = s.request("GET", "/rooms", None, None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["code"], code);
    let private = b["code"].as_str().unwrap().to_lowercase();
    let (status, res) = s
        .request("GET", &format!("/rooms/{private}"), None, None)
        .await;
    assert_eq!((status, &res["visibility"]), (200, &json!("private")));
    let (status, res) = s.request("GET", "/rooms/ZZZZZZ", None, None).await;
    assert_eq!((status, &res["code"]), (404, &json!("room_not_found")));

    // Con nombre provisional no se crea ni se entra.
    sqlx::query("UPDATE users SET display_name_set = false WHERE display_name = 'carla'")
        .execute(&pool)
        .await
        .unwrap();
    let (status, res) = s
        .create(
            &carla,
            json!({ "name": "Mesa", "visibility": "public",
                                                 "seats": 7, "preset": "vegas", "aids": false }),
        )
        .await;
    assert_eq!(
        (status, &res["code"]),
        (409, &json!("display_name_required"))
    );
    match s.try_connect(code, &carla).await {
        Err(WsError::Http(res)) => assert_eq!(res.status(), 409),
        other => panic!("debía rechazarse: {:?}", other.map(|_| ())),
    }
    // Sin sesión tampoco.
    match s.try_connect(code, "bj_session=nada").await {
        Err(WsError::Http(res)) => assert_eq!(res.status(), 401),
        other => panic!("debía rechazarse: {:?}", other.map(|_| ())),
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn room_round_with_two_players_and_a_spectator(pool: PgPool) {
    use Face::*;
    // Asiento 1 (Ana): 10,6; asiento 3 (Beto): 9,8; dealer 6 + oculta 10. Ana pide un 5
    // (21) y Beto se planta; el dealer pide con 16 y se pasa con un 9.
    let s = server(pool, fast(), &[Ten, Nine, Six, Six, Eight, Ten, Five, Nine]).await;
    let ana = s.register("ana").await;
    let beto = s.register("beto").await;
    let carla = s.register("carla").await;
    let code = s.room(&ana, false).await;

    let (mut a, ana_id) = s.connect(&code, &ana).await;
    let state = phase(&mut a, "waiting").await;
    assert_eq!(state["hostId"], ana_id);
    assert_eq!(state["seats"].as_array().unwrap().len(), 7);
    let (mut b, beto_id) = s.connect(&code, &beto).await;
    let (mut c, _) = s.connect(&code, &carla).await;

    send(&mut a, json!({ "type": "sit", "seat": 1 })).await;
    let state = phase(&mut a, "betting").await;
    assert_eq!(state["seats"][0]["player"]["displayName"], "ana");
    assert_eq!(state["seats"][0]["stack"], 500);
    send(&mut b, json!({ "type": "sit", "seat": 1 })).await;
    assert_eq!(error(&mut b).await, "seat_taken");
    send(&mut b, json!({ "type": "sit", "seat": 3 })).await;
    send(&mut b, json!({ "type": "bet", "amount": 15 })).await;
    assert_eq!(error(&mut b).await, "bet_out_of_range");
    send(&mut a, json!({ "type": "bet", "amount": 10 })).await;
    send(&mut b, json!({ "type": "bet", "amount": 20 })).await;

    // Apostaron todos: se reparte sin esperar el tiempo.
    let state = phase(&mut c, "playing").await;
    let round = &state["round"];
    assert_eq!(round["turn"], 1);
    assert_eq!(round["seats"][0]["hands"][0]["cards"][0]["rank"], "10");
    assert_eq!(round["seats"][1]["seat"], 3);
    assert_eq!(round["dealer"]["hole"], Value::Null, "la oculta no se ve");
    assert!(state.get("shoeCounts").is_none(), "sala sin ayudas");
    assert!(state["timerMs"].as_u64().unwrap() <= 5_000);
    assert_eq!(state["spectators"][0]["displayName"], "carla");

    send(&mut b, json!({ "type": "action", "action": "stand" })).await;
    assert_eq!(error(&mut b).await, "not_your_turn");
    send(&mut a, json!({ "type": "action", "action": "hit" })).await;
    let state = until(&mut b, |m| {
        m["type"] == "state" && m["room"]["round"]["turn"] == 3
    })
    .await;
    assert_eq!(state["room"]["round"]["seats"][0]["done"], true);
    send(&mut b, json!({ "type": "action", "action": "stand" })).await;

    let state = phase(&mut c, "results").await;
    let round = &state["round"];
    assert_eq!(round["dealer"]["hits"][0]["rank"], "9");
    assert_eq!(
        (&round["seats"][0]["net"], &round["seats"][1]["net"]),
        (&json!(10), &json!(20))
    );
    assert_eq!(
        (&state["seats"][0]["stack"], &state["seats"][2]["stack"]),
        (&json!(510), &json!(520))
    );
    let board = &state["leaderboard"];
    assert_eq!(board[0]["player"]["id"], beto_id);
    assert_eq!(
        (&board[0]["net"], &board[0]["hands"], &board[0]["buyIns"]),
        (&json!(20), &json!(1), &json!(1))
    );
    assert_eq!(board[1]["net"], 10);

    // Después de los resultados, otra vez apuestas.
    phase(&mut a, "betting").await;
}

#[sqlx::test(migrations = "./migrations")]
async fn room_timeouts_inactivity_and_rebuy(pool: PgPool) {
    use Face::*;
    let rooms = RoomsConfig {
        bet_time: Duration::from_millis(300),
        turn_time: Duration::from_millis(300),
        inactive_rounds: 2,
        ..fast()
    };
    // Ana (asiento 1): 10,7 contra 10 + oculta 10: pierde todo lo que apueste.
    let s = server(pool, rooms, &[Ten, Ten, Seven, Ten, Two, Two, Two]).await;
    let ana = s.register("ana").await;
    let beto = s.register("beto").await;
    let code = s.room(&ana, true).await;
    let (mut a, _) = s.connect(&code, &ana).await;
    let (mut b, _) = s.connect(&code, &beto).await;

    send(&mut a, json!({ "type": "sit", "seat": 1 })).await;
    send(&mut b, json!({ "type": "sit", "seat": 2 })).await;
    // Con Beto sentado, la apuesta de Ana sola no reparte: hay que esperar el tiempo.
    until(&mut a, |m| {
        m["type"] == "state" && m["room"]["seats"][1]["player"].is_object()
    })
    .await;
    send(&mut a, json!({ "type": "bet", "amount": 500 })).await;
    send(&mut a, json!({ "type": "rebuy" })).await;
    assert_eq!(error(&mut a).await, "rebuy_not_needed");

    // Beto no apuesta: al vencer el tiempo se reparte solo a Ana. Con ayudas viene
    // shoeCounts.
    let state = phase(&mut a, "playing").await;
    assert_eq!(state["round"]["seats"].as_array().unwrap().len(), 1);
    assert_eq!(
        state["shoeCounts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap())
            .sum::<u64>(),
        312 - 3
    );
    // Ana no decide: el servidor se planta por ella y pierde la pila.
    let state = phase(&mut a, "results").await;
    assert_eq!(state["round"]["seats"][0]["hands"][0]["outcome"], "lose");
    assert_eq!(state["seats"][0]["stack"], 0);

    send(&mut a, json!({ "type": "rebuy" })).await;
    let state = until(&mut a, |m| {
        m["type"] == "state" && m["room"]["seats"][0]["stack"] == 500
    })
    .await;
    // Ana queda última: 500 − 2 × 500.
    let board = &state["room"]["leaderboard"];
    assert_eq!(board[0]["player"]["displayName"], "beto");
    assert_eq!(
        (&board[1]["net"], &board[1]["buyIns"]),
        (&json!(-500), &json!(2))
    );

    // Dos rondas sin apostar: Beto vuelve a espectador.
    let state = until(&mut a, |m| {
        m["type"] == "state" && m["room"]["seats"][1].get("player").is_none()
    })
    .await;
    assert_eq!(state["room"]["spectators"][0]["displayName"], "beto");
}

#[sqlx::test(migrations = "./migrations")]
async fn room_reactions_kick_and_spectator_limit(pool: PgPool) {
    let s = server(
        pool,
        RoomsConfig {
            max_spectators: 2,
            ..fast()
        },
        &[],
    )
    .await;
    let ana = s.register("ana").await;
    let beto = s.register("beto").await;
    let carla = s.register("carla").await;
    let dani = s.register("dani").await;
    let code = s.room(&ana, false).await;
    let (mut a, ana_id) = s.connect(&code, &ana).await;
    let (mut b, beto_id) = s.connect(&code, &beto).await;
    phase(&mut b, "waiting").await;

    send(&mut b, json!({ "type": "react", "reaction": "fire" })).await;
    let r = until(&mut a, |m| m["type"] == "reaction").await;
    assert_eq!(
        r,
        json!({ "type": "reaction", "playerId": beto_id, "reaction": "fire" })
    );
    send(&mut b, json!({ "type": "react", "reaction": "gg" })).await;
    assert_eq!(error(&mut b).await, "rate_limited");
    send(&mut b, json!({ "type": "nada" })).await;
    assert_eq!(error(&mut b).await, "bad_request");

    // Ana y Beto son espectadores: con 2 de máximo, Carla no entra.
    match s.try_connect(&code, &carla).await {
        Err(WsError::Http(res)) => assert_eq!(res.status(), 409),
        other => panic!("debía rechazarse: {:?}", other.map(|_| ())),
    }

    send(&mut b, json!({ "type": "kick", "playerId": ana_id })).await;
    assert_eq!(error(&mut b).await, "not_host");
    send(&mut a, json!({ "type": "kick", "playerId": beto_id })).await;
    let closed = until(&mut b, |m| m["type"] == "closed").await;
    assert_eq!(closed["reason"], "kicked");
    match s.try_connect(&code, &beto).await {
        Err(WsError::Http(res)) => assert_eq!(res.status(), 403),
        other => panic!("debía rechazarse: {:?}", other.map(|_| ())),
    }
    // Se liberó un lugar.
    let (mut d, _) = s.connect(&code, &dani).await;
    let state = phase(&mut d, "waiting").await;
    assert_eq!(state["spectators"].as_array().unwrap().len(), 2);
}

#[sqlx::test(migrations = "./migrations")]
async fn room_reconnection_and_host_transfer(pool: PgPool) {
    let s = server(pool, fast(), &[]).await;
    let ana = s.register("ana").await;
    let beto = s.register("beto").await;
    let code = s.room(&ana, false).await;
    let (mut a, _) = s.connect(&code, &ana).await;
    let (mut b, beto_id) = s.connect(&code, &beto).await;
    send(&mut a, json!({ "type": "sit", "seat": 1 })).await;
    send(&mut b, json!({ "type": "sit", "seat": 2 })).await;
    until(&mut b, |m| {
        m["type"] == "state" && m["room"]["seats"][0]["player"].is_object()
    })
    .await;

    // Ana se cae y vuelve a tiempo: conserva el asiento.
    drop(a);
    until(&mut b, |m| {
        m["type"] == "state" && m["room"]["seats"][0]["connected"] == false
    })
    .await;
    let (mut a, _) = s.connect(&code, &ana).await;
    let state = until(&mut a, |m| m["type"] == "state").await;
    assert_eq!(state["room"]["seats"][0]["player"]["displayName"], "ana");
    assert_eq!(state["room"]["seats"][0]["connected"], true);

    // Se cae y no vuelve: pierde el asiento y el anfitrión pasa a Beto.
    drop(a);
    let state = until(&mut b, |m| {
        m["type"] == "state" && m["room"]["seats"][0].get("player").is_none()
    })
    .await;
    assert_eq!(state["room"]["hostId"], beto_id);
    assert_eq!(state["room"]["info"]["host"], "beto");
    let (_, list) = s.request("GET", "/rooms", None, None).await;
    assert_eq!(
        (&list[0]["host"], &list[0]["seated"]),
        (&json!("beto"), &json!(1))
    );
    // Ahora Beto tiene una sala: no puede crear otra.
    let (status, _) = s
        .create(
            &beto,
            json!({ "name": "Otra", "visibility": "public",
                                              "seats": 7, "preset": "vegas", "aids": false }),
        )
        .await;
    assert_eq!(status, 409);
}
