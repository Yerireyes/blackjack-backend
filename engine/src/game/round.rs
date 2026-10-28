//! Una ronda del jugador contra el dealer, con las mismas reglas de mesa que el
//! front (`tableStore.ts`): orden de reparto, peek, acciones permitidas, división
//! de ases, turno del dealer y liquidación. Sin bots y sin IO.

use serde::{Deserialize, Serialize};

use super::card::{hand_value, is_pair, Card};
use super::contract::{
    AllowedActions, CardView, DealerView, HandOutcome, HandView, PlayerView, ReviewItem,
    RoundPhase as Phase,
};
use super::shoe::GameShoe;
use super::GameError;
use crate::dto::{Action, Allowed, AnalyzeHandRequest, AnalyzeHandResponse};
use crate::rank::{Rank, NUM_RANKS};
use crate::rules::{Rules, Surrender};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlayerHand {
    pub cards: Vec<Card>,
    pub from_split: bool,
    /// Mano que salió de dividir ases.
    pub split_aces: bool,
    pub doubled: bool,
    pub surrendered: bool,
    pub done: bool,
    pub outcome: Option<HandOutcome>,
    /// Resultado en unidades de la apuesta inicial.
    pub net: Option<f64>,
}

impl PlayerHand {
    fn new(cards: Vec<Card>) -> PlayerHand {
        PlayerHand {
            cards,
            from_split: false,
            split_aces: false,
            doubled: false,
            surrendered: false,
            done: false,
            outcome: None,
            net: None,
        }
    }

    fn locked_split_aces(&self, rules: &Rules) -> bool {
        self.split_aces && !rules.hit_split_aces
    }

    fn is_live(&self) -> bool {
        let v = hand_value(&self.cards, self.from_split);
        !self.surrendered && !v.bust && !v.blackjack
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Dealer {
    pub up: Card,
    pub hole: Card,
    pub hole_revealed: bool,
    pub hits: Vec<Card>,
}

impl Dealer {
    fn cards(&self) -> Vec<Card> {
        let mut cards = vec![self.up, self.hole];
        cards.extend(&self.hits);
        cards
    }

    fn must_hit(&self, rules: &Rules) -> bool {
        let v = hand_value(&self.cards(), false);
        !v.blackjack && !v.bust && !dealer_stands(rules, v.total, v.soft)
    }
}

/// El dealer se planta con 17 o más; con 17 suave pide solo con H17.
fn dealer_stands(rules: &Rules, total: u8, soft: bool) -> bool {
    total >= 18 || (total == 17 && !(soft && rules.dealer_hits_soft17))
}

/// Con peek, el dealer revisa la oculta si la visible es As o vale 10.
fn dealer_checks_hole(rules: &Rules, up: Rank) -> bool {
    rules.dealer_peeks && (up.is_ace() || up == Rank::TEN)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Round {
    pub hands: Vec<PlayerHand>,
    pub active: usize,
    pub dealer: Dealer,
    pub phase: Phase,
    /// El dealer tenía blackjack al revisar la oculta: la ronda terminó en el reparto.
    pub peek_blackjack: bool,
    /// Ranked: cada decisión evaluada con la información que tenía el jugador.
    #[serde(default)]
    pub decisions: Vec<ReviewItem>,
}

impl Round {
    /// Reparte jugador, dealer (visible), jugador y dealer (oculta). Si la ronda no
    /// requiere decisiones (peek con blackjack del dealer o natural del jugador), el
    /// dealer juega y se liquida en el acto.
    pub fn deal(shoe: &mut GameShoe, rules: &Rules) -> Result<Round, GameError> {
        let p1 = shoe.draw()?;
        let up = shoe.draw()?;
        let p2 = shoe.draw()?;
        let hole = shoe.draw()?;
        let mut hand = PlayerHand::new(vec![p1, p2]);
        if hand_value(&hand.cards, false).blackjack {
            hand.done = true;
        }
        let mut round = Round {
            hands: vec![hand],
            active: 0,
            dealer: Dealer {
                up,
                hole,
                hole_revealed: false,
                hits: Vec::new(),
            },
            phase: Phase::Player,
            peek_blackjack: false,
            decisions: Vec::new(),
        };
        let dealer_bj = hand_value(&[up, hole], false).blackjack;
        if dealer_checks_hole(rules, up.rank()) && dealer_bj {
            round.peek_blackjack = true;
            round.play_dealer(shoe, rules)?;
        } else if round.hands.iter().all(|h| h.done) {
            round.play_dealer(shoe, rules)?;
        }
        Ok(round)
    }

    /// Unidades de apuesta en juego: una por mano, dos si se dobló.
    pub fn staked_units(&self) -> u32 {
        self.hands
            .iter()
            .map(|h| if h.doubled { 2 } else { 1 })
            .sum()
    }

    /// Acciones de la mano activa. `spare`: unidades de apuesta extra que el saldo
    /// cubre; doblar y dividir necesitan una.
    pub fn allowed(&self, rules: &Rules, spare: u32) -> AllowedActions {
        let none = AllowedActions::default();
        if self.phase != Phase::Player {
            return none;
        }
        let Some(h) = self.hands.get(self.active) else {
            return none;
        };
        if h.done || h.cards.len() < 2 {
            return none;
        }
        let v = hand_value(&h.cards, h.from_split);
        if v.bust || v.blackjack {
            return none;
        }
        let two = h.cards.len() == 2;
        let locked = h.locked_split_aces(rules);
        let pair = is_pair(&h.cards);
        AllowedActions {
            hit: !locked && v.total < 21,
            stand: true,
            double: two
                && spare >= 1
                && !h.doubled
                && !locked
                && rules.can_double_total(v.total, v.soft)
                && (!h.from_split || rules.double_after_split),
            split: two
                && spare >= 1
                && pair.is_some()
                && self.hands.len() < rules.max_split_hands as usize
                && !(pair == Some(Rank::ACE) && h.split_aces && !rules.resplit_aces),
            surrender: two
                && rules.surrender == Surrender::Late
                && rules.dealer_peeks
                && !h.from_split
                && self.hands.len() == 1,
        }
    }

    /// La decisión pendiente tal como la ve el jugador, para `analyze_hand`: sus
    /// cartas, la visible del dealer y lo no visto (la oculta incluida). `None` si
    /// no hay nada que decidir.
    pub fn analysis_request(
        &self,
        rules: &Rules,
        shoe: &GameShoe,
        spare: u32,
    ) -> Option<AnalyzeHandRequest> {
        let allowed = self.allowed(rules, spare);
        if !allowed.stand {
            return None;
        }
        let h = &self.hands[self.active];
        Some(AnalyzeHandRequest {
            version: 1,
            rules: rules.clone(),
            shoe: self.unseen_counts(shoe),
            hand: h.cards.iter().map(|c| c.rank()).collect(),
            dealer_up: self.dealer.up.rank(),
            split_hands_so_far: (self.hands.len() - 1) as u8,
            is_split_aces: h.split_aces,
            allowed: Allowed {
                double: allowed.double,
                split: allowed.split,
                surrender: allowed.surrender,
            },
        })
    }

    /// Guarda la decisión `chosen` de la mano activa frente al análisis de
    /// [`Round::analysis_request`]. Llamar antes de [`Round::act`].
    pub fn record_decision(&mut self, chosen: Action, analysis: &AnalyzeHandResponse) {
        let ev = |a: Action| analysis.actions.iter().find(|r| r.action == a);
        let (Some(c), Some(b)) = (ev(chosen), ev(analysis.best)) else {
            return;
        };
        self.decisions.push(ReviewItem {
            hand_index: self.active as u32,
            chosen,
            best: analysis.best,
            ev_chosen: c.ev,
            ev_best: b.ev,
            exact: c.exact && b.exact,
        });
    }

    /// Aplica una acción del jugador. Si con ella terminan sus manos, juega el dealer
    /// y liquida.
    pub fn act(
        &mut self,
        action: Action,
        shoe: &mut GameShoe,
        rules: &Rules,
        spare: u32,
    ) -> Result<(), GameError> {
        if !self.allowed(rules, spare).get(action) {
            return Err(GameError::ActionNotAllowed);
        }
        let i = self.active;
        let h = &mut self.hands[i];
        match action {
            Action::Hit => {
                h.cards.push(shoe.draw()?);
                let v = hand_value(&h.cards, h.from_split);
                if v.bust || v.total == 21 {
                    h.done = true;
                }
            }
            Action::Stand => h.done = true,
            Action::Double => {
                h.doubled = true;
                h.cards.push(shoe.draw()?);
                h.done = true;
            }
            Action::Surrender => {
                h.surrendered = true;
                h.done = true;
            }
            Action::Split => {
                let (a, b) = (h.cards[0], h.cards[1]);
                let aces = a.rank().is_ace();
                let half = |c: Card| PlayerHand {
                    from_split: true,
                    split_aces: aces,
                    ..PlayerHand::new(vec![c])
                };
                let (mut h1, h2) = (half(a), half(b));
                h1.cards.push(shoe.draw()?);
                if h1.locked_split_aces(rules) || hand_value(&h1.cards, true).total == 21 {
                    h1.done = true;
                }
                self.hands.splice(i..=i, [h1, h2]);
            }
        }
        if self.hands[self.active].done {
            self.advance(shoe, rules)?;
        }
        Ok(())
    }

    /// Pasa a la siguiente mano pendiente (dándole su segunda carta si viene de
    /// dividir) o, si no queda ninguna, al turno del dealer.
    fn advance(&mut self, shoe: &mut GameShoe, rules: &Rules) -> Result<(), GameError> {
        for i in self.active..self.hands.len() {
            let h = &mut self.hands[i];
            if h.done {
                continue;
            }
            self.active = i;
            if h.cards.len() == 1 {
                h.cards.push(shoe.draw()?);
                if h.locked_split_aces(rules) || hand_value(&h.cards, true).total == 21 {
                    h.done = true;
                    continue;
                }
            }
            return Ok(());
        }
        self.active = self.active.min(self.hands.len() - 1);
        self.play_dealer(shoe, rules)
    }

    /// Revela la oculta, pide si alguna mano sigue viva y liquida.
    fn play_dealer(&mut self, shoe: &mut GameShoe, rules: &Rules) -> Result<(), GameError> {
        self.dealer.hole_revealed = true;
        if self.hands.iter().any(PlayerHand::is_live) {
            while self.dealer.must_hit(rules) {
                self.dealer.hits.push(shoe.draw()?);
            }
        }
        let dealer = self.dealer.cards();
        for h in &mut self.hands {
            let (outcome, net) = settle_hand(h, &dealer, rules);
            h.outcome = Some(outcome);
            h.net = Some(net);
            h.done = true;
        }
        self.phase = Phase::Settled;
        Ok(())
    }

    /// Neto de la ronda en unidades de la apuesta inicial (solo liquidada).
    pub fn net_units(&self) -> f64 {
        self.hands.iter().filter_map(|h| h.net).sum()
    }

    /// Neto en fichas: `round(neto × apuesta)`, igual que el front.
    pub fn net_chips(&self, bet: i64) -> i64 {
        chips(self.net_units(), bet)
    }

    /// Cartas sin ver para el jugador: lo que queda en el zapato más la oculta
    /// mientras no se revela.
    pub fn unseen_counts(&self, shoe: &GameShoe) -> [u16; NUM_RANKS] {
        let mut counts = shoe.remaining_counts();
        if !self.dealer.hole_revealed {
            counts[self.dealer.hole.rank().index()] += 1;
        }
        counts
    }

    /// Lo que el jugador puede ver de su mano y del dealer.
    pub fn player_view(&self, bet: i64) -> PlayerView {
        PlayerView {
            hands: self
                .hands
                .iter()
                .map(|h| HandView {
                    cards: h.cards.iter().map(|&c| CardView::from(c)).collect(),
                    doubled: h.doubled,
                    surrendered: h.surrendered,
                    from_split: h.from_split,
                    outcome: h.outcome,
                    net: h.net.map(|n| chips(n, bet)),
                })
                .collect(),
            active: self.active as u32,
        }
    }

    pub fn dealer_view(&self) -> DealerView {
        DealerView {
            up: self.dealer.up.into(),
            hole: self.dealer.hole_revealed.then(|| self.dealer.hole.into()),
            hits: self.dealer.hits.iter().map(|&c| c.into()).collect(),
        }
    }
}

/// `round(x)` como `Math.round` de JavaScript (los medios redondean hacia arriba).
fn chips(units: f64, bet: i64) -> i64 {
    (units * bet as f64 + 0.5).floor() as i64
}

/// Liquida una mano frente al dealer, en unidades de la apuesta inicial.
pub fn settle_hand(h: &PlayerHand, dealer: &[Card], rules: &Rules) -> (HandOutcome, f64) {
    let bet = if h.doubled { 2.0 } else { 1.0 };
    let dv = hand_value(dealer, false);
    let pv = hand_value(&h.cards, h.from_split);
    if h.surrendered {
        return (HandOutcome::Surrender, -0.5);
    }
    if pv.blackjack {
        return if dv.blackjack {
            (HandOutcome::Push, 0.0)
        } else {
            (HandOutcome::Blackjack, rules.blackjack_pays)
        };
    }
    // Sin peek (regla europea) el blackjack del dealer se lleva todo lo apostado.
    if dv.blackjack {
        return (HandOutcome::Lose, -bet);
    }
    if pv.bust {
        return (HandOutcome::Bust, -bet);
    }
    if dv.bust || pv.total > dv.total {
        return (HandOutcome::Win, bet);
    }
    if pv.total == dv.total {
        return (HandOutcome::Push, 0.0);
    }
    (HandOutcome::Lose, -bet)
}
