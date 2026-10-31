//! El actor de una sala: una tarea de Tokio dueña de todo su estado (asientos, pilas,
//! zapato, ronda, temporizadores, ranking interno y torneo). Recibe órdenes por un canal
//! y manda el estado completo a cada conexión cada vez que cambia algo. Nada se guarda en
//! la base salvo el logro del ganador de un torneo: el ranking interno muere con la sala.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use engine::game::{
    ClientMessage, CosmeticId, GameError, GameShoe, RoomInfo, RoomLeaderboardRow, RoomPhase,
    RoomPlayer, RoomSeat, RoomState, RoundPhase, ServerMessage, TableRound, TournamentStanding,
    TournamentStatus, TournamentView,
};
use engine::{Action, Rules};
use sqlx::PgPool;
use tokio::sync::{mpsc, oneshot};
use tokio::time::{sleep_until, Instant};
use uuid::Uuid;

use super::{RoomsConfig, ShoeFactory, Summary};
use crate::routes::casual::{BET_STEP, MAX_BET, MIN_BET};

/// Fichas al sentarse por primera vez y al recomprar.
pub const STACK: i64 = 500;

/// Lo que el actor manda a una conexión.
#[derive(Debug)]
pub enum Out {
    Text(Arc<str>),
    /// Cerrar la conexión (después de un `closed`).
    Close,
}

pub type ConnTx = mpsc::Sender<Out>;

/// Por qué no puede entrar alguien.
#[derive(Debug, PartialEq, Eq)]
pub enum JoinError {
    Banned,
    Full,
}

pub enum Command {
    /// ¿Puede entrar? (antes de abrir el WebSocket, para responder con un código HTTP).
    Admit {
        user: Uuid,
        reply: oneshot::Sender<Result<(), JoinError>>,
    },
    Join {
        user: Uuid,
        display_name: String,
        title: Option<CosmeticId>,
        conn: u64,
        tx: ConnTx,
    },
    Leave {
        user: Uuid,
        conn: u64,
    },
    Client {
        user: Uuid,
        msg: ClientMessage,
    },
}

struct Member {
    player: RoomPlayer,
    conns: HashMap<u64, ConnTx>,
    joined_at: Instant,
    /// Desde cuándo no tiene conexiones.
    gone_since: Option<Instant>,
    last_reaction: Option<Instant>,
}

/// La pila de un usuario en la sala; se conserva si se levanta.
struct Wallet {
    player: RoomPlayer,
    stack: i64,
    buy_ins: u32,
    hands: u32,
}

#[derive(Default, Clone)]
struct Seat {
    user: Option<Uuid>,
    /// Rondas seguidas sin apostar.
    missed: u32,
    /// Se va cuando termine la ronda (lo echaron o se cayó y no volvió).
    leaving: bool,
}

enum Phase {
    Waiting,
    Betting {
        deadline: Instant,
        bets: HashMap<u8, i64>,
    },
    Playing {
        round: TableRound,
        deadline: Instant,
    },
    Results {
        round: TableRound,
        deadline: Instant,
    },
}

/// El torneo de una sala de torneo: pilas propias de 500 para los que estaban sentados al
/// empezar, sin recompras, durante `hands` manos.
struct Tournament {
    hands: u32,
    played: u32,
    status: TournamentStatus,
    stacks: HashMap<Uuid, i64>,
    winner: Option<Uuid>,
}

pub struct Room {
    code: String,
    info: RoomInfo,
    rules: Rules,
    host: Uuid,
    cfg: RoomsConfig,
    shoe_factory: ShoeFactory,
    shoe: GameShoe,
    members: HashMap<Uuid, Member>,
    seats: Vec<Seat>,
    wallets: HashMap<Uuid, Wallet>,
    banned: HashSet<Uuid>,
    phase: Phase,
    /// Solo en las salas de torneo.
    tournament: Option<Tournament>,
    /// Para el logro del ganador de un torneo.
    pool: Option<PgPool>,
    /// Desde cuándo no hay nadie conectado.
    empty_since: Option<Instant>,
    summary: Arc<RwLock<Summary>>,
    dirty: bool,
    closed: bool,
}

/// Una orden rechazada; se manda como `error` y la conexión sigue.
struct Reject {
    code: &'static str,
    message: String,
}

impl From<Reject> for ServerMessage {
    fn from(r: Reject) -> ServerMessage {
        ServerMessage::Error {
            code: r.code.into(),
            message: r.message,
        }
    }
}

fn error(code: &'static str, message: &str) -> Reject {
    Reject {
        code,
        message: message.into(),
    }
}

fn text(msg: &ServerMessage) -> Arc<str> {
    serde_json::to_string(msg)
        .expect("los mensajes se serializan")
        .into()
}

impl Room {
    pub fn new(
        code: String,
        info: RoomInfo,
        host: Uuid,
        cfg: RoomsConfig,
        shoe_factory: ShoeFactory,
        summary: Arc<RwLock<Summary>>,
        pool: Option<PgPool>,
    ) -> Room {
        let rules = info.rules.clone();
        Room {
            shoe: shoe_factory(rules.decks),
            seats: vec![Seat::default(); info.seats as usize],
            tournament: info.tournament.map(|t| Tournament {
                hands: t.hands as u32,
                played: 0,
                status: TournamentStatus::Waiting,
                stacks: HashMap::new(),
                winner: None,
            }),
            pool,
            code,
            info,
            rules,
            host,
            cfg,
            shoe_factory,
            members: HashMap::new(),
            wallets: HashMap::new(),
            banned: HashSet::new(),
            phase: Phase::Waiting,
            empty_since: Some(Instant::now()),
            summary,
            dirty: false,
            closed: false,
        }
    }

    /// El bucle del actor. Termina cuando la sala se cierra.
    pub async fn run(mut self, mut rx: mpsc::Receiver<Command>) {
        while !self.closed {
            let wake = self.next_wake();
            tokio::select! {
                cmd = rx.recv() => match cmd {
                    Some(cmd) => self.handle(cmd),
                    None => break,
                },
                _ = sleep_until(wake) => self.tick(Instant::now()),
            }
            if self.dirty {
                self.dirty = false;
                self.publish();
            }
        }
        self.close_all("closed");
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    // ---- Tiempo ----

    fn next_wake(&self) -> Instant {
        let far = Instant::now() + std::time::Duration::from_secs(3600);
        let mut wake = far;
        let mut at = |t: Instant| wake = wake.min(t);
        match &self.phase {
            Phase::Waiting => {}
            Phase::Betting { deadline, .. }
            | Phase::Playing { deadline, .. }
            | Phase::Results { deadline, .. } => at(*deadline),
        }
        for m in self.members.values() {
            if let Some(t) = m.gone_since {
                at(t + self.cfg.reconnect);
            }
        }
        if let Some(t) = self.empty_since {
            at(t + self.cfg.empty_close);
        }
        wake
    }

    fn tick(&mut self, now: Instant) {
        if self
            .empty_since
            .is_some_and(|t| now >= t + self.cfg.empty_close)
        {
            self.closed = true;
            return;
        }
        // Desconectados por más del margen: pierden el asiento (y el anfitrión, el rol).
        let gone: Vec<Uuid> = self
            .members
            .iter()
            .filter(|(_, m)| m.gone_since.is_some_and(|t| now >= t + self.cfg.reconnect))
            .map(|(u, _)| *u)
            .collect();
        for user in gone {
            self.release_seat_of(user);
            self.members.remove(&user);
            if user == self.host {
                self.transfer_host();
            }
            self.dirty = true;
        }
        enum Due {
            Deal,
            TimeOut(u8),
            /// Se acabó el tiempo del seguro: quien no decidió, no lo toma.
            NoInsurance(Vec<u8>),
            Next,
        }
        let due = match &self.phase {
            Phase::Betting { deadline, .. } if now >= *deadline => Some(Due::Deal),
            Phase::Playing { deadline, round } if now >= *deadline => {
                if round.phase == RoundPhase::Insurance {
                    Some(Due::NoInsurance(round.pending_insurance()))
                } else {
                    round.turn_seat().map(Due::TimeOut)
                }
            }
            Phase::Results { deadline, .. } if now >= *deadline => Some(Due::Next),
            _ => None,
        };
        match due {
            Some(Due::Deal) => self.start_round(),
            // Se acabó el tiempo: el servidor se planta por el jugador.
            Some(Due::TimeOut(seat)) => {
                let _ = self.play(seat, Action::Stand);
            }
            Some(Due::NoInsurance(seats)) => {
                for seat in seats {
                    let _ = self.insure(seat, false);
                }
            }
            Some(Due::Next) => self.after_results(),
            None => {}
        }
    }

    // ---- Órdenes ----

    fn handle(&mut self, cmd: Command) {
        match cmd {
            Command::Admit { user, reply } => {
                let _ = reply.send(self.admit(user));
            }
            Command::Join {
                user,
                display_name,
                title,
                conn,
                tx,
            } => {
                if let Err(e) = self.admit(user) {
                    let reason = if e == JoinError::Banned {
                        "banned"
                    } else {
                        "full"
                    };
                    let _ = tx.try_send(Out::Text(text(&ServerMessage::Closed {
                        reason: reason.into(),
                    })));
                    let _ = tx.try_send(Out::Close);
                    return;
                }
                let player = RoomPlayer {
                    id: user.to_string(),
                    display_name,
                    title,
                };
                let welcome = text(&ServerMessage::Welcome {
                    player_id: player.id.clone(),
                });
                let _ = tx.try_send(Out::Text(welcome));
                let member = self.members.entry(user).or_insert_with(|| Member {
                    player: player.clone(),
                    conns: HashMap::new(),
                    joined_at: Instant::now(),
                    gone_since: None,
                    last_reaction: None,
                });
                member.player = player;
                member.conns.insert(conn, tx);
                member.gone_since = None;
                self.empty_since = None;
                if let Some(i) = self.seat_index_of(user) {
                    self.seats[i].leaving = false;
                }
                self.dirty = true;
            }
            Command::Leave { user, conn } => {
                if let Some(m) = self.members.get_mut(&user) {
                    m.conns.remove(&conn);
                    if m.conns.is_empty() {
                        m.gone_since = Some(Instant::now());
                    }
                }
                if self.members.values().all(|m| m.conns.is_empty()) {
                    self.empty_since = Some(Instant::now());
                }
                self.dirty = true;
            }
            Command::Client { user, msg } => {
                if let Err(e) = self.client(user, msg) {
                    self.send_to(user, &e.into());
                }
            }
        }
    }

    fn admit(&self, user: Uuid) -> Result<(), JoinError> {
        if self.banned.contains(&user) {
            return Err(JoinError::Banned);
        }
        let known = self.members.contains_key(&user) || self.seat_index_of(user).is_some();
        if !known && self.spectator_count() >= self.cfg.max_spectators as usize {
            return Err(JoinError::Full);
        }
        Ok(())
    }

    fn client(&mut self, user: Uuid, msg: ClientMessage) -> Result<(), Reject> {
        match msg {
            ClientMessage::Sit { seat } => self.sit(user, seat),
            ClientMessage::LeaveSeat => {
                let i = self
                    .seat_index_of(user)
                    .ok_or_else(|| error("not_seated", "no estás sentado"))?;
                if self.in_round(i) {
                    return Err(error(
                        "round_in_progress",
                        "espera a que termine la ronda para levantarte",
                    ));
                }
                self.free_seat(i);
                Ok(())
            }
            ClientMessage::Bet { amount } => self.bet(user, amount),
            ClientMessage::Action { action } => {
                let i = self
                    .seat_index_of(user)
                    .ok_or_else(|| error("not_seated", "no estás sentado"))?;
                if !matches!(self.phase, Phase::Playing { .. }) {
                    return Err(error("not_your_turn", "no es tu turno"));
                }
                self.play(i as u8 + 1, action)
            }
            ClientMessage::Insurance { take } => {
                let i = self
                    .seat_index_of(user)
                    .ok_or_else(|| error("not_seated", "no estás sentado"))?;
                self.insure(i as u8 + 1, take)
            }
            ClientMessage::StartTournament => self.start_tournament(user),
            ClientMessage::Rebuy => {
                if self.tournament.is_some() {
                    return Err(error("tournament_no_rebuy", "en un torneo no se recompra"));
                }
                let i = self.seat_index_of(user);
                if i.is_some_and(|i| self.in_round(i)) {
                    return Err(error("round_in_progress", "espera a que termine la ronda"));
                }
                let w = self
                    .wallets
                    .get_mut(&user)
                    .ok_or_else(|| error("not_seated", "todavía no te sentaste"))?;
                if w.stack >= MIN_BET {
                    return Err(error(
                        "rebuy_not_needed",
                        "solo se recompra con menos de 10 fichas",
                    ));
                }
                w.stack = STACK;
                w.buy_ins += 1;
                self.dirty = true;
                Ok(())
            }
            ClientMessage::React { reaction } => {
                let now = Instant::now();
                let m = self
                    .members
                    .get_mut(&user)
                    .ok_or_else(|| error("not_in_room", "no estás en la sala"))?;
                if m.last_reaction
                    .is_some_and(|t| now < t + self.cfg.reaction_every)
                {
                    return Err(error("rate_limited", "espera un poco para reaccionar"));
                }
                m.last_reaction = Some(now);
                let msg = ServerMessage::Reaction {
                    player_id: user.to_string(),
                    reaction,
                };
                self.broadcast(&msg);
                Ok(())
            }
            ClientMessage::Kick { player_id } => {
                if user != self.host {
                    return Err(error("not_host", "solo el anfitrión puede echar"));
                }
                let target: Uuid = player_id
                    .parse()
                    .map_err(|_| error("bad_request", "playerId inválido"))?;
                if target == user {
                    return Err(error("bad_request", "no te puedes echar a ti mismo"));
                }
                if !self.members.contains_key(&target) && self.seat_index_of(target).is_none() {
                    return Err(error("not_in_room", "no está en la sala"));
                }
                self.banned.insert(target);
                if let Some(m) = self.members.remove(&target) {
                    let closed = text(&ServerMessage::Closed {
                        reason: "kicked".into(),
                    });
                    for tx in m.conns.values() {
                        let _ = tx.try_send(Out::Text(closed.clone()));
                        let _ = tx.try_send(Out::Close);
                    }
                }
                self.release_seat_of(target);
                self.dirty = true;
                Ok(())
            }
        }
    }

    fn sit(&mut self, user: Uuid, seat: u8) -> Result<(), Reject> {
        if seat == 0 || seat as usize > self.seats.len() {
            return Err(error("bad_request", "ese asiento no existe"));
        }
        if self.seat_index_of(user).is_some() {
            return Err(error("already_seated", "ya estás sentado"));
        }
        let i = seat as usize - 1;
        if self.seats[i].user.is_some() {
            return Err(error("seat_taken", "ese asiento está ocupado"));
        }
        if self.tournament_running() {
            return Err(error(
                "tournament_running",
                "el torneo ya empezó: espera al siguiente",
            ));
        }
        let player = self
            .members
            .get(&user)
            .map(|m| m.player.clone())
            .ok_or_else(|| error("not_in_room", "no estás en la sala"))?;
        let wallet = self.wallets.entry(user).or_insert_with(|| Wallet {
            player: player.clone(),
            stack: STACK,
            buy_ins: 1,
            hands: 0,
        });
        wallet.player = player;
        self.seats[i] = Seat {
            user: Some(user),
            missed: 0,
            leaving: false,
        };
        // En una sala de torneo solo se juega con el torneo en marcha.
        if matches!(self.phase, Phase::Waiting) && self.tournament.is_none() {
            self.start_betting();
        }
        self.dirty = true;
        Ok(())
    }

    // ---- Torneos ----

    fn tournament_running(&self) -> bool {
        self.tournament
            .as_ref()
            .is_some_and(|t| t.status == TournamentStatus::Running)
    }

    /// La pila con la que juega el usuario: la del torneo en una sala de torneo (500
    /// mientras no empezó) o la suya de la sala.
    fn stack(&self, user: Uuid) -> i64 {
        match &self.tournament {
            Some(t) => t.stacks.get(&user).copied().unwrap_or(STACK),
            None => self.wallets.get(&user).map_or(0, |w| w.stack),
        }
    }

    fn stack_mut(&mut self, user: Uuid) -> Option<&mut i64> {
        match &mut self.tournament {
            Some(t) => t.stacks.get_mut(&user),
            None => self.wallets.get_mut(&user).map(|w| &mut w.stack),
        }
    }

    /// El anfitrión inicia un torneo con los que están sentados (2 o más).
    fn start_tournament(&mut self, user: Uuid) -> Result<(), Reject> {
        if user != self.host {
            return Err(error("not_host", "solo el anfitrión inicia el torneo"));
        }
        let seated: Vec<Uuid> = self.seats.iter().filter_map(|s| s.user).collect();
        let Some(t) = &mut self.tournament else {
            return Err(error("not_a_tournament", "esta sala no es de torneo"));
        };
        if t.status == TournamentStatus::Running {
            return Err(error("tournament_running", "el torneo ya está en marcha"));
        }
        if seated.len() < 2 {
            return Err(error(
                "not_enough_players",
                "hacen falta al menos 2 jugadores sentados",
            ));
        }
        t.status = TournamentStatus::Running;
        t.played = 0;
        t.winner = None;
        t.stacks = seated.into_iter().map(|u| (u, STACK)).collect();
        for s in &mut self.seats {
            s.missed = 0;
        }
        self.start_betting();
        Ok(())
    }

    /// Termina si se jugaron todas las manos o queda uno solo (o nadie) con fichas
    /// sentado. Gana la pila más alta; el ganador recibe el logro `tournamentWin`.
    fn maybe_finish_tournament(&mut self) {
        let seated: HashSet<Uuid> = self.seats.iter().filter_map(|s| s.user).collect();
        let Some(t) = &mut self.tournament else {
            return;
        };
        if t.status != TournamentStatus::Running {
            return;
        }
        let alive = t
            .stacks
            .iter()
            .filter(|(u, &s)| s >= MIN_BET && seated.contains(u))
            .count();
        if t.played < t.hands && alive >= 2 {
            return;
        }
        t.status = TournamentStatus::Finished;
        let wallets = &self.wallets;
        let name = |u: &Uuid| wallets.get(u).map(|w| w.player.display_name.clone());
        t.winner = t
            .stacks
            .iter()
            .max_by(|a, b| a.1.cmp(b.1).then_with(|| name(b.0).cmp(&name(a.0))))
            .map(|(u, _)| *u);
        if let (Some(winner), Some(pool)) = (t.winner, self.pool.clone()) {
            tokio::spawn(async move {
                let res = sqlx::query!(
                    "INSERT INTO user_achievements (user_id, achievement)
                     VALUES ($1, 'tournamentWin') ON CONFLICT DO NOTHING",
                    winner
                )
                .execute(&pool)
                .await;
                if let Err(e) = res {
                    tracing::warn!("no se pudo otorgar el logro del torneo: {e}");
                }
            });
        }
        self.dirty = true;
    }

    fn tournament_view(&self) -> Option<TournamentView> {
        let t = self.tournament.as_ref()?;
        let seated: HashSet<Uuid> = self.seats.iter().filter_map(|s| s.user).collect();
        let player = |u: &Uuid| {
            self.wallets
                .get(u)
                .map(|w| w.player.clone())
                .unwrap_or_else(|| RoomPlayer {
                    id: u.to_string(),
                    display_name: String::new(),
                    title: None,
                })
        };
        let mut rows: Vec<(Uuid, i64)> = t.stacks.iter().map(|(u, s)| (*u, *s)).collect();
        rows.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| player(&a.0).display_name.cmp(&player(&b.0).display_name))
        });
        let standings = rows
            .iter()
            .map(|(u, stack)| TournamentStanding {
                rank: 1 + rows.iter().filter(|(_, s)| s > stack).count() as u32,
                player: player(u),
                stack: *stack,
                out: *stack < MIN_BET || !seated.contains(u),
            })
            .collect();
        Some(TournamentView {
            status: t.status,
            hands: t.hands,
            played: t.played,
            standings,
            winner: t.winner.map(|u| player(&u)),
        })
    }

    fn bet(&mut self, user: Uuid, amount: i64) -> Result<(), Reject> {
        let i = self
            .seat_index_of(user)
            .ok_or_else(|| error("not_seated", "no estás sentado"))?;
        let stack = self.stack(user);
        let Phase::Betting { bets, .. } = &mut self.phase else {
            return Err(error("betting_closed", "ahora no se puede apostar"));
        };
        if !(MIN_BET..=MAX_BET).contains(&amount) || amount % BET_STEP != 0 {
            return Err(error(
                "bet_out_of_range",
                &format!("la apuesta debe ser de {MIN_BET} a {MAX_BET} en múltiplos de {BET_STEP}"),
            ));
        }
        if amount > stack {
            return Err(error(
                "insufficient_funds",
                "no te alcanza la pila para esa apuesta",
            ));
        }
        bets.insert(i as u8 + 1, amount);
        let bets_len = bets.len();
        self.dirty = true;
        // Si ya apostaron todos los que están conectados, se reparte sin esperar.
        let ready = self
            .seats
            .iter()
            .filter(|s| s.user.is_some_and(|u| self.connected(u)))
            .count();
        if bets_len >= ready {
            self.start_round();
        }
        Ok(())
    }

    // ---- La ronda ----

    fn start_betting(&mut self) {
        self.phase = Phase::Betting {
            deadline: Instant::now() + self.cfg.bet_time,
            bets: HashMap::new(),
        };
        self.dirty = true;
    }

    fn start_round(&mut self) {
        let Phase::Betting { bets, .. } = std::mem::replace(&mut self.phase, Phase::Waiting) else {
            return;
        };
        // En un torneo cada cierre de apuestas es una mano jugada (así siempre termina);
        // quien no apuesta solo se la pierde.
        if let Some(t) = self.tournament.as_mut() {
            if t.status == TournamentStatus::Running {
                t.played += 1;
            }
        }
        // Quien no apostó suma una ronda perdida; a las 3 vuelve a espectador (no en las
        // salas de torneo).
        for i in 0..self.seats.len() {
            if self.seats[i].user.is_none() || self.tournament.is_some() {
                continue;
            }
            if bets.contains_key(&(i as u8 + 1)) {
                self.seats[i].missed = 0;
            } else {
                self.seats[i].missed += 1;
                if self.seats[i].missed >= self.cfg.inactive_rounds {
                    self.free_seat(i);
                }
            }
        }
        let bets: Vec<(u8, i64)> = bets
            .into_iter()
            .filter(|(seat, _)| self.seats[*seat as usize - 1].user.is_some())
            .collect();
        self.dirty = true;
        if bets.is_empty() {
            self.after_results();
            return;
        }
        let low = (self
            .shoe
            .remaining_counts()
            .iter()
            .map(|&c| c as usize)
            .sum::<usize>())
            < 12 * bets.len() + 10;
        if low
            || self
                .shoe
                .needs_shuffle(self.rules.penetration, self.rules.max_split_hands)
        {
            self.shoe = (self.shoe_factory)(self.rules.decks);
        }
        match TableRound::deal(&mut self.shoe, &self.rules, &bets) {
            Ok(round) => self.continue_round(round),
            Err(_) => self.void_round(),
        }
    }

    /// Tras un reparto o una acción: si la ronda terminó, liquida; si no, reinicia el
    /// tiempo del turno (o se planta enseguida por quien se va).
    fn continue_round(&mut self, round: TableRound) {
        if round.phase == RoundPhase::Settled {
            self.settle(round);
            return;
        }
        let leaving = round
            .turn_seat()
            .is_some_and(|s| self.seats[s as usize - 1].leaving);
        // Quien se va no toma el seguro.
        let declines: Vec<u8> = round
            .pending_insurance()
            .into_iter()
            .filter(|&s| self.seats[s as usize - 1].leaving)
            .collect();
        self.phase = Phase::Playing {
            round,
            deadline: Instant::now() + self.cfg.turn_time,
        };
        for seat in declines {
            let _ = self.insure(seat, false);
        }
        let turn = match &self.phase {
            Phase::Playing { round, .. } if leaving => round.turn_seat(),
            _ => None,
        };
        if let Some(seat) = turn {
            let _ = self.play(seat, Action::Stand);
        }
        self.dirty = true;
    }

    /// El asiento `seat` toma o rechaza el seguro. Mientras falten decisiones el tiempo
    /// del seguro no se reinicia; cuando decidieron todos, sigue la ronda.
    fn insure(&mut self, seat: u8, take: bool) -> Result<(), Reject> {
        let stack = self.seats[seat as usize - 1].user.map(|u| self.stack(u));
        let Phase::Playing { round, .. } = &mut self.phase else {
            return Err(error("action_not_allowed", "ahora no se ofrece el seguro"));
        };
        // Fichas de la pila fuera de la apuesta (en el seguro todavía no hay dobles).
        let free = stack
            .zip(round.seats.iter().find(|s| s.seat == seat))
            .map_or(0, |(stack, s)| stack - s.bet);
        match round.insure(seat, take, free, &mut self.shoe, &self.rules) {
            Ok(()) if round.phase == RoundPhase::Insurance => {
                self.dirty = true;
                Ok(())
            }
            Ok(()) => {
                let Phase::Playing { round, .. } =
                    std::mem::replace(&mut self.phase, Phase::Waiting)
                else {
                    unreachable!()
                };
                self.continue_round(round);
                Ok(())
            }
            Err(GameError::NotYourTurn) => {
                Err(error("not_in_round", "no tienes mano en esta ronda"))
            }
            Err(GameError::ActionNotAllowed) => Err(error(
                "action_not_allowed",
                "el seguro ya se decidió, no se ofrece o no te alcanza la pila",
            )),
            Err(GameError::ShoeEmpty) => {
                self.void_round();
                Ok(())
            }
        }
    }

    /// Una acción del asiento `seat` (1..), que tiene que estar en turno.
    fn play(&mut self, seat: u8, action: Action) -> Result<(), Reject> {
        let stack = self.seats[seat as usize - 1].user.map(|u| self.stack(u));
        let Phase::Playing { round, .. } = &mut self.phase else {
            return Err(error("not_your_turn", "no es tu turno"));
        };
        let spare = stack
            .zip(round.seats.iter().find(|s| s.seat == seat))
            .map_or(0, |(stack, s)| {
                spare_units(stack - s.insurance_chips(), s.bet, s.staked_units())
            });
        match round.act(seat, action, &mut self.shoe, &self.rules, spare) {
            Ok(()) => {
                let Phase::Playing { round, .. } =
                    std::mem::replace(&mut self.phase, Phase::Waiting)
                else {
                    unreachable!()
                };
                self.continue_round(round);
                Ok(())
            }
            Err(GameError::NotYourTurn) => Err(error("not_your_turn", "no es tu turno")),
            Err(GameError::ActionNotAllowed) => Err(error(
                "action_not_allowed",
                "esa acción no está permitida ahora",
            )),
            Err(GameError::ShoeEmpty) => {
                self.void_round();
                Ok(())
            }
        }
    }

    fn settle(&mut self, round: TableRound) {
        for s in &round.seats {
            let Some(user) = self.seats[s.seat as usize - 1].user else {
                continue;
            };
            let net = s.net_chips(&round.dealer);
            if let Some(stack) = self.stack_mut(user) {
                *stack = (*stack + net).max(0);
            }
            if let Some(w) = self.wallets.get_mut(&user) {
                w.hands += 1;
            }
        }
        self.phase = Phase::Results {
            round,
            deadline: Instant::now() + self.cfg.results_time,
        };
        self.dirty = true;
    }

    /// El zapato se acabó en mitad de la ronda: se anula (las apuestas no se habían
    /// cobrado) y se rebaraja.
    fn void_round(&mut self) {
        self.shoe = (self.shoe_factory)(self.rules.decks);
        let voided = error("round_voided", "se acabó el zapato: la ronda se anuló");
        self.broadcast(&voided.into());
        self.after_results();
    }

    fn after_results(&mut self) {
        for i in 0..self.seats.len() {
            if self.seats[i].leaving {
                self.free_seat(i);
            }
        }
        self.maybe_finish_tournament();
        let playing = self.tournament.is_none() || self.tournament_running();
        if playing && self.seats.iter().any(|s| s.user.is_some()) {
            self.start_betting();
        } else {
            self.phase = Phase::Waiting;
        }
        self.dirty = true;
    }

    // ---- Asientos y miembros ----

    fn seat_index_of(&self, user: Uuid) -> Option<usize> {
        self.seats.iter().position(|s| s.user == Some(user))
    }

    fn connected(&self, user: Uuid) -> bool {
        self.members.get(&user).is_some_and(|m| !m.conns.is_empty())
    }

    /// El asiento `i` tiene mano en la ronda en juego.
    fn in_round(&self, i: usize) -> bool {
        match &self.phase {
            Phase::Playing { round, .. } => round.seats.iter().any(|s| s.seat as usize == i + 1),
            _ => false,
        }
    }

    fn free_seat(&mut self, i: usize) {
        self.seats[i] = Seat::default();
        if let Phase::Betting { bets, .. } = &mut self.phase {
            bets.remove(&(i as u8 + 1));
        }
        self.dirty = true;
    }

    /// Libera el asiento del usuario ahora o, si su mano está en juego, al terminar la
    /// ronda (mientras tanto el servidor se planta por él).
    fn release_seat_of(&mut self, user: Uuid) {
        let Some(i) = self.seat_index_of(user) else {
            return;
        };
        let playing = matches!(&self.phase, Phase::Playing { round, .. }
            if round.seats.iter().any(|s| s.seat as usize == i + 1));
        if playing {
            self.seats[i].leaving = true;
            let seat = i as u8 + 1;
            let (turn, insuring) = match &self.phase {
                Phase::Playing { round, .. } => {
                    (round.turn_seat(), round.pending_insurance().contains(&seat))
                }
                _ => (None, false),
            };
            if insuring {
                let _ = self.insure(seat, false);
            } else if turn == Some(seat) {
                let _ = self.play(seat, Action::Stand);
            }
        } else {
            self.free_seat(i);
        }
    }

    /// El anfitrión pasa al sentado más antiguo en la sala o, si no hay, al espectador
    /// más antiguo.
    fn transfer_host(&mut self) {
        let seated: HashSet<Uuid> = self.seats.iter().filter_map(|s| s.user).collect();
        let oldest = |only_seated: bool| {
            self.members
                .iter()
                .filter(|(u, m)| !m.conns.is_empty() && (!only_seated || seated.contains(u)))
                .min_by_key(|(_, m)| m.joined_at)
                .map(|(u, _)| *u)
        };
        if let Some(next) = oldest(true).or_else(|| oldest(false)) {
            self.host = next;
            if let Some(m) = self.members.get(&next) {
                self.info.host = m.player.display_name.clone();
            }
        }
    }

    fn spectator_count(&self) -> usize {
        self.members
            .iter()
            .filter(|(u, m)| !m.conns.is_empty() && self.seat_index_of(**u).is_none())
            .count()
    }

    // ---- Envío ----

    fn send_to(&mut self, user: Uuid, msg: &ServerMessage) {
        let t = text(msg);
        if let Some(m) = self.members.get_mut(&user) {
            m.conns
                .retain(|_, tx| tx.try_send(Out::Text(t.clone())).is_ok());
        }
    }

    /// A todas las conexiones. Una conexión que no da abasto se corta.
    fn broadcast(&mut self, msg: &ServerMessage) {
        let t = text(msg);
        for m in self.members.values_mut() {
            m.conns
                .retain(|_, tx| tx.try_send(Out::Text(t.clone())).is_ok());
            if m.conns.is_empty() && m.gone_since.is_none() {
                m.gone_since = Some(Instant::now());
            }
        }
    }

    fn close_all(&mut self, reason: &str) {
        let t = text(&ServerMessage::Closed {
            reason: reason.into(),
        });
        for m in self.members.values() {
            for tx in m.conns.values() {
                let _ = tx.try_send(Out::Text(t.clone()));
                let _ = tx.try_send(Out::Close);
            }
        }
        self.members.clear();
    }

    fn publish(&mut self) {
        self.info.seated = self.seats.iter().filter(|s| s.user.is_some()).count() as u8;
        self.info.spectators = self.spectator_count() as u8;
        if let Ok(mut s) = self.summary.write() {
            s.info = self.info.clone();
            s.host = self.host;
            s.members = self
                .members
                .iter()
                .filter(|(_, m)| !m.conns.is_empty())
                .map(|(u, _)| *u)
                .collect();
        }
        let state = self.state();
        self.broadcast(&ServerMessage::State { room: state });
    }

    fn state(&self) -> RoomState {
        let now = Instant::now();
        let (phase, deadline, round, bets) = match &self.phase {
            Phase::Waiting => (RoomPhase::Waiting, None, None, None),
            Phase::Betting { deadline, bets } => {
                (RoomPhase::Betting, Some(*deadline), None, Some(bets))
            }
            Phase::Playing { round, deadline } => {
                (RoomPhase::Playing, Some(*deadline), Some(round), None)
            }
            Phase::Results { round, deadline } => {
                (RoomPhase::Results, Some(*deadline), Some(round), None)
            }
        };
        let seats = self
            .seats
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let wallet = s.user.and_then(|u| self.wallets.get(&u));
                RoomSeat {
                    seat: i as u8 + 1,
                    player: wallet.map(|w| w.player.clone()),
                    stack: s.user.map_or(0, |u| self.stack(u)),
                    connected: s.user.is_some_and(|u| self.connected(u)),
                    bet: bets.and_then(|b| b.get(&(i as u8 + 1)).copied()),
                }
            })
            .collect();
        let seated: HashSet<Uuid> = self.seats.iter().filter_map(|s| s.user).collect();
        let mut spectators: Vec<(&Member, &Uuid)> = self
            .members
            .iter()
            .filter(|(u, m)| !m.conns.is_empty() && !seated.contains(u))
            .map(|(u, m)| (m, u))
            .collect();
        spectators.sort_by_key(|(m, _)| m.joined_at);
        let round_view = round.map(|r| {
            let spare = r
                .turn_seat()
                .and_then(|seat| {
                    let stack = self.seats[seat as usize - 1].user.map(|u| self.stack(u))?;
                    let s = r.seats.iter().find(|s| s.seat == seat)?;
                    Some(spare_units(
                        stack - s.insurance_chips(),
                        s.bet,
                        s.staked_units(),
                    ))
                })
                .unwrap_or(0);
            r.view(&self.rules, spare)
        });
        let shoe_counts = self.info.aids.then(|| match round {
            Some(r) => r.unseen_counts(&self.shoe),
            None => self.shoe.remaining_counts(),
        });
        // En una sala de torneo manda la clasificación del torneo.
        let mut leaderboard: Vec<RoomLeaderboardRow> = self
            .wallets
            .values()
            .filter(|_| self.tournament.is_none())
            .map(|w| RoomLeaderboardRow {
                player: w.player.clone(),
                net: w.stack - STACK * w.buy_ins as i64,
                hands: w.hands,
                buy_ins: w.buy_ins,
            })
            .collect();
        leaderboard.sort_by(|a, b| {
            b.net
                .cmp(&a.net)
                .then(b.hands.cmp(&a.hands))
                .then(a.player.display_name.cmp(&b.player.display_name))
        });
        RoomState {
            info: self.info.clone(),
            host_id: self.host.to_string(),
            seats,
            spectators: spectators
                .into_iter()
                .map(|(m, _)| m.player.clone())
                .collect(),
            phase,
            timer_ms: deadline.map(|d| d.saturating_duration_since(now).as_millis() as u32),
            round: round_view,
            shoe_counts,
            penetration: self.shoe.dealt_fraction(),
            leaderboard,
            tournament: self.tournament_view(),
        }
    }
}

/// Unidades extra que cubre la pila (doblar y dividir necesitan una).
fn spare_units(stack: i64, bet: i64, staked: u32) -> u32 {
    let free = stack - bet * staked as i64;
    (free.max(0) / bet).min(u32::MAX as i64) as u32
}
