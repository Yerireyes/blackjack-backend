//! Salas Casual multijugador: mesas de 5 a 7 asientos contra el dealer, públicas o
//! privadas, en tiempo real por WebSocket. Cada sala es un actor en memoria
//! ([`room::Room`]); aquí están el registro de salas, sus límites y las rutas.
//! Especificación: «docs/Especificación — Salas Casual.md».

pub mod room;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use axum::Json;
use engine::game::{
    ClientMessage, CosmeticId, CreateRoomRequest, GameShoe, RoomInfo, RoomVisibility, ServerMessage,
};
use futures_util::{SinkExt, StreamExt};
use rand::RngExt;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::extract::{ApiJson, ApiPath};
use crate::routes::casual::{now_micros, require_display_name, rfc3339, shuffled_shoe};
use crate::routes::cosmetics::title;
use crate::state::AppState;
use room::{Command, JoinError, Out, Room};

/// Fabrica el zapato barajado de una sala (los tests ponen uno armado).
pub type ShoeFactory = Arc<dyn Fn(u8) -> GameShoe + Send + Sync>;

/// Límites y tiempos de las salas.
#[derive(Clone, Debug)]
pub struct RoomsConfig {
    /// Salas abiertas a la vez (`MAX_ROOMS`).
    pub max_rooms: usize,
    pub max_spectators: u8,
    pub bet_time: Duration,
    pub turn_time: Duration,
    pub results_time: Duration,
    /// Cuánto conserva el asiento (y el rol de anfitrión) un desconectado.
    pub reconnect: Duration,
    /// Sala sin conexiones: se cierra.
    pub empty_close: Duration,
    pub reaction_every: Duration,
    /// Rondas seguidas sin apostar para volver a espectador.
    pub inactive_rounds: u32,
    /// Mensajes por segundo de una conexión; más, y se corta.
    pub max_msgs_per_sec: u32,
}

impl RoomsConfig {
    pub fn new(max_rooms: usize) -> RoomsConfig {
        RoomsConfig {
            max_rooms,
            max_spectators: 10,
            bet_time: Duration::from_secs(20),
            turn_time: Duration::from_secs(20),
            results_time: Duration::from_secs(5),
            reconnect: Duration::from_secs(60),
            empty_close: Duration::from_secs(120),
            reaction_every: Duration::from_secs(2),
            inactive_rounds: 3,
            max_msgs_per_sec: 20,
        }
    }
}

/// Lo que el lobby sabe de una sala; lo actualiza el actor.
pub struct Summary {
    pub info: RoomInfo,
    pub host: Uuid,
    /// Usuarios con al menos una conexión abierta (para amigos e invitaciones).
    pub members: Vec<Uuid>,
}

struct Handle {
    tx: mpsc::Sender<Command>,
    summary: Arc<RwLock<Summary>>,
}

pub struct Rooms {
    map: Arc<Mutex<HashMap<String, Handle>>>,
    pub cfg: RoomsConfig,
    shoe: ShoeFactory,
    next_conn: AtomicU64,
}

/// Sin letras ni números que se confundan (0/O, 1/I/L).
const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";

fn new_code() -> String {
    let mut rng = rand::rng();
    (0..6)
        .map(|_| CODE_ALPHABET[rng.random_range(0..CODE_ALPHABET.len())] as char)
        .collect()
}

impl Rooms {
    pub fn new(cfg: RoomsConfig) -> Rooms {
        Rooms::with_shoe(cfg, Arc::new(shuffled_shoe))
    }

    pub fn with_shoe(cfg: RoomsConfig, shoe: ShoeFactory) -> Rooms {
        Rooms {
            map: Arc::new(Mutex::new(HashMap::new())),
            cfg,
            shoe,
            next_conn: AtomicU64::new(1),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Handle>> {
        self.map.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Crea la sala y arranca su actor.
    fn create(&self, mut info: RoomInfo, host: Uuid) -> ApiResult<RoomInfo> {
        let mut map = self.lock();
        if map
            .values()
            .any(|h| h.summary.read().is_ok_and(|s| s.host == host))
        {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "room_hosting",
                "ya tienes una sala abierta",
            ));
        }
        if map.len() >= self.cfg.max_rooms {
            return Err(ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "rooms_full",
                "no hay lugar para más salas ahora; prueba en un rato",
            ));
        }
        let code = loop {
            let code = new_code();
            if !map.contains_key(&code) {
                break code;
            }
        };
        info.code = code.clone();
        let summary = Arc::new(RwLock::new(Summary {
            info: info.clone(),
            host,
            members: Vec::new(),
        }));
        let (tx, rx) = mpsc::channel(256);
        let room = Room::new(
            code.clone(),
            info.clone(),
            host,
            self.cfg.clone(),
            self.shoe.clone(),
            summary.clone(),
        );
        map.insert(code.clone(), Handle { tx, summary });
        let registry = self.map.clone();
        tokio::spawn(async move {
            room.run(rx).await;
            registry
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&code);
        });
        Ok(info)
    }

    fn find(&self, code: &str) -> Option<(mpsc::Sender<Command>, RoomInfo)> {
        let map = self.lock();
        let h = map.get(&code.to_ascii_uppercase())?;
        let info = h.summary.read().ok()?.info.clone();
        Some((h.tx.clone(), info))
    }

    fn public(&self) -> Vec<RoomInfo> {
        let map = self.lock();
        let mut rooms: Vec<RoomInfo> = map
            .values()
            .filter_map(|h| h.summary.read().ok().map(|s| s.info.clone()))
            .filter(|i| i.visibility == RoomVisibility::Public)
            .collect();
        // Primero las que tienen asientos libres; entre ellas, las más llenas.
        rooms.sort_by_key(|i| {
            (
                i.seated >= i.seats,
                std::cmp::Reverse(i.seated),
                i.created_at.clone(),
            )
        });
        rooms.truncate(100);
        rooms
    }

    /// Salas abiertas ahora.
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// La sala abierta (por código) si existe.
    pub fn info(&self, code: &str) -> Option<RoomInfo> {
        self.find(code).map(|(_, info)| info)
    }

    /// ¿El usuario está conectado a esa sala?
    pub fn is_member(&self, code: &str, user: Uuid) -> bool {
        let map = self.lock();
        map.get(&code.to_ascii_uppercase())
            .and_then(|h| h.summary.read().ok().map(|s| s.members.contains(&user)))
            .unwrap_or(false)
    }

    /// Una sala pública en la que está conectado el usuario, si hay.
    pub fn public_room_of(&self, user: Uuid) -> Option<RoomInfo> {
        let map = self.lock();
        map.values()
            .filter_map(|h| h.summary.read().ok())
            .find(|s| s.info.visibility == RoomVisibility::Public && s.members.contains(&user))
            .map(|s| s.info.clone())
    }

    /// Salas abiertas, jugadores sentados y espectadores, para las métricas.
    pub fn stats(&self) -> (usize, usize, usize) {
        let map = self.lock();
        let (seated, spectators) = map
            .values()
            .filter_map(|h| {
                h.summary
                    .read()
                    .ok()
                    .map(|s| (s.info.seated, s.info.spectators))
            })
            .fold((0, 0), |(a, b), (s, e)| (a + s as usize, b + e as usize));
        (map.len(), seated, spectators)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn room_not_found() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "room_not_found",
        "esa sala no existe o ya cerró",
    )
}

/// Nombre visible y título del usuario.
async fn player_of(state: &AppState, user: Uuid) -> ApiResult<(String, Option<CosmeticId>)> {
    let mut db = state.pool.acquire().await?;
    require_display_name(&mut db, user).await?;
    let r = sqlx::query!(
        r#"SELECT u.display_name, c.title AS "title?"
           FROM users u LEFT JOIN user_cosmetics c ON c.user_id = u.id
           WHERE u.id = $1"#,
        user
    )
    .fetch_one(&mut *db)
    .await?;
    Ok((r.display_name, title(r.title.as_deref())))
}

/// `POST /rooms`: crea una sala; quien la crea es el anfitrión.
pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    ApiJson(req): ApiJson<CreateRoomRequest>,
) -> ApiResult<(StatusCode, Json<RoomInfo>)> {
    let name = req.name.trim().to_string();
    let len = name.chars().count();
    if !(3..=30).contains(&len) || name.chars().any(char::is_control) {
        return Err(ApiError::bad_request(
            "el nombre de la sala debe tener de 3 a 30 caracteres",
        ));
    }
    if !(5..=7).contains(&req.seats) {
        return Err(ApiError::bad_request("la mesa tiene 5, 6 o 7 asientos"));
    }
    let (rules, preset) = match (req.preset, req.rules) {
        (Some(p), None) => (p.rules(), Some(p)),
        (None, Some(r)) => (r, None),
        _ => {
            return Err(ApiError::bad_request(
                "indica un preset o reglas personalizadas (uno de los dos)",
            ))
        }
    };
    rules.validate()?;
    let (host_name, _) = player_of(&state, user.id).await?;
    let info = RoomInfo {
        code: String::new(),
        name,
        visibility: req.visibility,
        host: host_name,
        seats: req.seats,
        seated: 0,
        spectators: 0,
        max_spectators: state.rooms.cfg.max_spectators,
        rules,
        preset,
        aids: req.aids,
        created_at: rfc3339(now_micros()),
    };
    let info = state.rooms.create(info, user.id)?;
    Ok((StatusCode::CREATED, Json(info)))
}

/// `GET /rooms`: las salas públicas (hasta 100).
pub async fn list(State(state): State<AppState>) -> Json<Vec<RoomInfo>> {
    Json(state.rooms.public())
}

/// `GET /rooms/{code}`: una sala, también privada (con el código se entra).
pub async fn get(
    State(state): State<AppState>,
    ApiPath(code): ApiPath<String>,
) -> ApiResult<Json<RoomInfo>> {
    let (_, info) = state.rooms.find(&code).ok_or_else(room_not_found)?;
    Ok(Json(info))
}

/// `GET /rooms/{code}/ws`: el WebSocket de la sala. Se entra como espectador.
pub async fn ws(
    State(state): State<AppState>,
    user: AuthUser,
    ApiPath(code): ApiPath<String>,
    upgrade: WebSocketUpgrade,
) -> ApiResult<Response> {
    let (room, _) = state.rooms.find(&code).ok_or_else(room_not_found)?;
    let (display_name, title) = player_of(&state, user.id).await?;
    let (reply, admitted) = oneshot::channel();
    room.send(Command::Admit {
        user: user.id,
        reply,
    })
    .await
    .map_err(|_| room_not_found())?;
    match admitted.await.map_err(|_| room_not_found())? {
        Ok(()) => {}
        Err(JoinError::Banned) => {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "room_banned",
                "el anfitrión te sacó de esta sala",
            ))
        }
        Err(JoinError::Full) => {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "room_full",
                "la sala no admite más espectadores",
            ))
        }
    }
    let conn = state.rooms.next_conn.fetch_add(1, Ordering::Relaxed);
    let max_msgs = state.rooms.cfg.max_msgs_per_sec;
    Ok(upgrade.on_upgrade(move |socket| {
        connection(socket, room, user.id, conn, display_name, title, max_msgs)
    }))
}

/// Una conexión: reenvía lo que manda el actor y le pasa los mensajes del cliente.
async fn connection(
    socket: WebSocket,
    room: mpsc::Sender<Command>,
    user: Uuid,
    conn: u64,
    display_name: String,
    title: Option<CosmeticId>,
    max_msgs: u32,
) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::channel::<Out>(64);
    let errors = tx.clone();
    if room
        .send(Command::Join {
            user,
            display_name,
            title,
            conn,
            tx,
        })
        .await
        .is_err()
    {
        return;
    }
    let writer = tokio::spawn(async move {
        // Un ping cada 30 s para que proxies y PaaS no corten la conexión.
        let mut ping = tokio::time::interval(Duration::from_secs(30));
        ping.tick().await;
        loop {
            tokio::select! {
                out = rx.recv() => match out {
                    Some(Out::Text(t)) => {
                        if sink.send(Message::Text(t.as_ref().into())).await.is_err() {
                            break;
                        }
                    }
                    Some(Out::Close) | None => break,
                },
                _ = ping.tick() => {
                    if sink.send(Message::Ping(Vec::new().into())).await.is_err() {
                        break;
                    }
                }
            }
        }
        let _ = sink.close().await;
    });

    let mut window = tokio::time::Instant::now();
    let mut count = 0;
    loop {
        let msg = tokio::select! {
            msg = stream.next() => msg,
            // El actor cerró la conexión (la echaron o la sala cerró).
            _ = errors.closed() => None,
        };
        let Some(Ok(msg)) = msg else { break };
        let Message::Text(raw) = msg else {
            if matches!(msg, Message::Close(_)) {
                break;
            }
            continue;
        };
        let now = tokio::time::Instant::now();
        if now >= window + Duration::from_secs(1) {
            window = now;
            count = 0;
        }
        count += 1;
        if count > max_msgs {
            break;
        }
        match serde_json::from_str::<ClientMessage>(&raw) {
            Ok(msg) => {
                if room.send(Command::Client { user, msg }).await.is_err() {
                    break;
                }
            }
            Err(e) => {
                let err = ServerMessage::Error {
                    code: "bad_request".into(),
                    message: format!("mensaje inválido: {e}"),
                };
                if let Ok(t) = serde_json::to_string(&err) {
                    let _ = errors.try_send(Out::Text(t.into()));
                }
            }
        }
        if writer.is_finished() {
            break;
        }
    }
    let _ = room.send(Command::Leave { user, conn }).await;
    drop(errors);
    let _ = writer.await;
}
