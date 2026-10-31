//! Una ronda de una sala: varios asientos contra el mismo dealer y el mismo zapato.
//! Reparte como en el casino (una carta a cada asiento, la visible del dealer, la
//! segunda a cada asiento y la oculta) y juega los turnos asiento por asiento. Cada mano
//! sigue las mismas reglas que [`super::Round`]: las funciones por mano son compartidas.

use serde::{Deserialize, Serialize};

use super::card::hand_value;
use super::contract::{AllowedActions, RoundPhase as Phase, SeatRoundView, TableRoundView};
use super::round::{
    advance_hands, apply_action, chips, dealer_checks_hole, hand_view, hands_allowed,
    insurance_cost, insurance_net, play_dealer, settle_hands, Dealer, PlayerHand,
};
use super::shoe::GameShoe;
use super::GameError;
use crate::dto::Action;
use crate::rank::NUM_RANKS;
use crate::rules::Rules;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TableSeat {
    /// Número de asiento, desde 1.
    pub seat: u8,
    pub bet: i64,
    pub hands: Vec<PlayerHand>,
    pub active: usize,
    /// Seguro tomado o rechazado (`None`: no se ofreció o falta decidir).
    #[serde(default)]
    pub insurance: Option<bool>,
}

impl TableSeat {
    fn done(&self) -> bool {
        self.hands.iter().all(|h| h.done)
    }

    /// Unidades de apuesta en juego: una por mano, dos si se dobló.
    pub fn staked_units(&self) -> u32 {
        self.hands
            .iter()
            .map(|h| if h.doubled { 2 } else { 1 })
            .sum()
    }

    /// Fichas puestas en el seguro.
    pub fn insurance_chips(&self) -> i64 {
        if self.insurance == Some(true) {
            insurance_cost(self.bet)
        } else {
            0
        }
    }

    /// Neto en fichas, seguro incluido (solo liquidada).
    pub fn net_chips(&self, dealer: &Dealer) -> i64 {
        let insurance = if self.insurance == Some(true) {
            insurance_net(dealer, self.bet)
        } else {
            0
        };
        chips(self.hands.iter().filter_map(|h| h.net).sum(), self.bet) + insurance
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TableRound {
    /// Los asientos que apostaron, en orden.
    pub seats: Vec<TableSeat>,
    /// Índice en `seats` del que decide.
    pub turn: Option<usize>,
    pub dealer: Dealer,
    pub phase: Phase,
    pub peek_blackjack: bool,
}

impl TableRound {
    /// Reparte a los asientos `bets` (asiento, apuesta), que se ordenan por asiento. Si
    /// nadie tiene que decidir (peek con blackjack del dealer o todos con blackjack), el
    /// dealer juega y se liquida en el acto.
    pub fn deal(
        shoe: &mut GameShoe,
        rules: &Rules,
        bets: &[(u8, i64)],
    ) -> Result<TableRound, GameError> {
        let mut bets = bets.to_vec();
        bets.sort_by_key(|&(seat, _)| seat);
        let first = bets
            .iter()
            .map(|_| shoe.draw())
            .collect::<Result<Vec<_>, _>>()?;
        let up = shoe.draw()?;
        let second = bets
            .iter()
            .map(|_| shoe.draw())
            .collect::<Result<Vec<_>, _>>()?;
        let hole = shoe.draw()?;
        let seats = bets
            .iter()
            .zip(first.into_iter().zip(second))
            .map(|(&(seat, bet), (a, b))| {
                let mut hand = PlayerHand::new(vec![a, b]);
                if hand_value(&hand.cards, false).blackjack {
                    hand.done = true;
                }
                TableSeat {
                    seat,
                    bet,
                    hands: vec![hand],
                    active: 0,
                    insurance: None,
                }
            })
            .collect();
        let mut round = TableRound {
            seats,
            turn: None,
            dealer: Dealer {
                up,
                hole,
                hole_revealed: false,
                hits: Vec::new(),
            },
            phase: Phase::Player,
            peek_blackjack: false,
        };
        if up.rank().is_ace() {
            round.phase = Phase::Insurance;
        } else {
            round.after_deal(shoe, rules)?;
        }
        Ok(round)
    }

    /// Peek y, si no termina ahí, el primer turno.
    fn after_deal(&mut self, shoe: &mut GameShoe, rules: &Rules) -> Result<(), GameError> {
        let (up, hole) = (self.dealer.up, self.dealer.hole);
        if dealer_checks_hole(rules, up.rank()) && hand_value(&[up, hole], false).blackjack {
            self.peek_blackjack = true;
            self.finish(shoe, rules)
        } else {
            self.next_turn(0, shoe, rules)
        }
    }

    /// Asientos que todavía no decidieron el seguro (solo en la fase `insurance`).
    pub fn pending_insurance(&self) -> Vec<u8> {
        if self.phase != Phase::Insurance {
            return Vec::new();
        }
        self.seats
            .iter()
            .filter(|s| s.insurance.is_none())
            .map(|s| s.seat)
            .collect()
    }

    /// El asiento `seat` decide el seguro. `free_chips`: fichas de su pila fuera de la
    /// apuesta. Cuando decidieron todos, sigue la ronda (peek y turnos).
    pub fn insure(
        &mut self,
        seat: u8,
        take: bool,
        free_chips: i64,
        shoe: &mut GameShoe,
        rules: &Rules,
    ) -> Result<(), GameError> {
        if self.phase != Phase::Insurance {
            return Err(GameError::ActionNotAllowed);
        }
        let Some(s) = self.seats.iter_mut().find(|s| s.seat == seat) else {
            return Err(GameError::NotYourTurn);
        };
        if s.insurance.is_some() || (take && free_chips < insurance_cost(s.bet)) {
            return Err(GameError::ActionNotAllowed);
        }
        s.insurance = Some(take);
        if self.seats.iter().all(|s| s.insurance.is_some()) {
            self.phase = Phase::Player;
            self.after_deal(shoe, rules)?;
        }
        Ok(())
    }

    /// El asiento al que le toca decidir.
    pub fn turn_seat(&self) -> Option<u8> {
        self.turn.map(|i| self.seats[i].seat)
    }

    /// Acciones del asiento en turno. `spare`: unidades extra que su pila cubre.
    pub fn allowed(&self, rules: &Rules, spare: u32) -> AllowedActions {
        match self.turn {
            Some(i) if self.phase == Phase::Player => {
                let s = &self.seats[i];
                hands_allowed(&s.hands, s.active, rules, spare)
            }
            _ => AllowedActions::default(),
        }
    }

    /// Aplica la acción del asiento `seat`, que tiene que estar en turno. Si con ella
    /// termina, pasa al siguiente; tras el último juega el dealer y se liquida.
    pub fn act(
        &mut self,
        seat: u8,
        action: Action,
        shoe: &mut GameShoe,
        rules: &Rules,
        spare: u32,
    ) -> Result<(), GameError> {
        let Some(i) = self.turn.filter(|&i| self.seats[i].seat == seat) else {
            return Err(GameError::NotYourTurn);
        };
        if !self.allowed(rules, spare).get(action) {
            return Err(GameError::ActionNotAllowed);
        }
        let s = &mut self.seats[i];
        if apply_action(&mut s.hands, &mut s.active, action, shoe, rules)? {
            self.next_turn(i + 1, shoe, rules)?;
        }
        Ok(())
    }

    /// Pone en turno al primer asiento desde `from` que tenga algo que decidir (dándole
    /// la segunda carta a una mano dividida si hace falta). Si no queda ninguno, juega el
    /// dealer.
    fn next_turn(
        &mut self,
        from: usize,
        shoe: &mut GameShoe,
        rules: &Rules,
    ) -> Result<(), GameError> {
        for i in from..self.seats.len() {
            let s = &mut self.seats[i];
            if s.done() || advance_hands(&mut s.hands, &mut s.active, shoe, rules)? {
                continue;
            }
            self.turn = Some(i);
            return Ok(());
        }
        self.finish(shoe, rules)
    }

    fn finish(&mut self, shoe: &mut GameShoe, rules: &Rules) -> Result<(), GameError> {
        self.turn = None;
        let live = self
            .seats
            .iter()
            .any(|s| s.hands.iter().any(PlayerHand::is_live));
        play_dealer(&mut self.dealer, live, shoe, rules)?;
        for s in &mut self.seats {
            settle_hands(&mut s.hands, &self.dealer, rules);
        }
        self.phase = Phase::Settled;
        Ok(())
    }

    /// Cartas sin ver para la mesa: lo que queda en el zapato más la oculta mientras no
    /// se revela.
    pub fn unseen_counts(&self, shoe: &GameShoe) -> [u16; NUM_RANKS] {
        let mut counts = shoe.remaining_counts();
        if !self.dealer.hole_revealed {
            counts[self.dealer.hole.rank().index()] += 1;
        }
        counts
    }

    /// Lo que ve la mesa. `spare`: unidades extra que cubre la pila del asiento en turno.
    pub fn view(&self, rules: &Rules, spare: u32) -> TableRoundView {
        let settled = self.phase == Phase::Settled;
        TableRoundView {
            phase: self.phase,
            seats: self
                .seats
                .iter()
                .map(|s| SeatRoundView {
                    seat: s.seat,
                    bet: s.bet,
                    hands: s.hands.iter().map(|h| hand_view(h, s.bet)).collect(),
                    active: s.active as u32,
                    done: s.done(),
                    insured: s.insurance,
                    net: settled.then(|| s.net_chips(&self.dealer)),
                })
                .collect(),
            dealer: self.dealer.view(),
            turn: self.turn_seat(),
            allowed: self.allowed(rules, spare),
            peek_blackjack: self.peek_blackjack,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::card::Face::{self, *};
    use crate::game::contract::{CardView, HandOutcome, Preset};

    const PLENTY: u32 = 100;

    fn vegas() -> Rules {
        Preset::Vegas.rules()
    }

    fn deal(top: &[Face], bets: &[(u8, i64)]) -> (TableRound, GameShoe) {
        let rules = vegas();
        let mut shoe = GameShoe::stacked(rules.decks, top);
        let round = TableRound::deal(&mut shoe, &rules, bets).unwrap();
        (round, shoe)
    }

    fn faces(cards: &[CardView]) -> Vec<Face> {
        cards.iter().map(|c| c.rank).collect()
    }

    #[test]
    fn deals_like_a_casino_and_plays_seats_in_order() {
        // Asientos 2 y 5 (pedidos al revés). Primera vuelta: 10 (s2), 9 (s5); dealer 6;
        // segunda: 6 (s2), 8 (s5); oculta 10.
        let (mut r, mut shoe) = deal(
            &[Ten, Nine, Six, Six, Eight, Ten, Five, Nine],
            &[(5, 20), (2, 10)],
        );
        let rules = vegas();
        let v = r.view(&rules, PLENTY);
        assert_eq!(v.seats.iter().map(|s| s.seat).collect::<Vec<_>>(), [2, 5]);
        assert_eq!(faces(&v.seats[0].hands[0].cards), [Ten, Six]);
        assert_eq!(faces(&v.seats[1].hands[0].cards), [Nine, Eight]);
        assert_eq!(v.dealer.up.rank, Six);
        assert_eq!(v.dealer.hole, None, "la oculta no se ve");
        assert_eq!(v.turn, Some(2));
        // 312 − 5 visibles: la oculta cuenta como no vista.
        assert_eq!(r.unseen_counts(&shoe).iter().sum::<u16>(), 312 - 5);

        assert_eq!(
            r.act(5, Action::Stand, &mut shoe, &rules, PLENTY),
            Err(GameError::NotYourTurn)
        );
        r.act(2, Action::Hit, &mut shoe, &rules, PLENTY).unwrap(); // 10,6 + 5 = 21: termina solo
        assert_eq!(r.turn_seat(), Some(5));
        r.act(5, Action::Stand, &mut shoe, &rules, PLENTY).unwrap();
        // Dealer 6 + 10 = 16, pide un 9 y se pasa.
        assert_eq!(r.phase, Phase::Settled);
        assert_eq!(r.turn_seat(), None);
        let v = r.view(&rules, PLENTY);
        assert_eq!(faces(&v.dealer.hits), [Nine]);
        assert_eq!((v.seats[0].net, v.seats[1].net), (Some(10), Some(20)));
    }

    #[test]
    fn blackjacks_skip_their_turn_and_peek_ends_the_round() {
        let rules = vegas();
        // s1: A,K (blackjack); s3: 10,7; dealer 9 + 7.
        let (r, _) = deal(&[Ace, Ten, Nine, K, Seven, Seven], &[(1, 10), (3, 10)]);
        assert_eq!(r.turn_seat(), Some(3), "el blackjack no juega");
        assert!(r.seats[0].hands[0].done);

        // Dealer As + K con peek: tras el seguro termina en el reparto para todos.
        let (mut r, mut shoe) = deal(&[Ten, Nine, Ace, Seven, Eight, K], &[(1, 10), (2, 30)]);
        assert_eq!(r.phase, Phase::Insurance);
        for seat in [1, 2] {
            r.insure(seat, false, 100, &mut shoe, &rules).unwrap();
        }
        assert!(r.peek_blackjack);
        assert_eq!(r.phase, Phase::Settled);
        let v = r.view(&rules, PLENTY);
        assert_eq!((v.seats[0].net, v.seats[1].net), (Some(-10), Some(-30)));
        assert!(v.dealer.hole.is_some());
    }

    #[test]
    fn split_hands_get_their_second_card_on_their_turn() {
        let rules = vegas();
        // s1: 8,8; s2: 10,9; dealer 6 + 10. Dividir: el primer 8 recibe un 3.
        let (mut r, mut shoe) = deal(
            &[Eight, Ten, Six, Eight, Nine, Ten, Three, Two, Seven],
            &[(1, 10), (2, 10)],
        );
        r.act(1, Action::Split, &mut shoe, &rules, PLENTY).unwrap();
        assert_eq!(r.seats[0].hands.len(), 2);
        assert_eq!(
            faces(&r.view(&rules, PLENTY).seats[0].hands[0].cards),
            [Eight, Three]
        );
        r.act(1, Action::Stand, &mut shoe, &rules, PLENTY).unwrap();
        // Sigue el mismo asiento con su segunda mano, que recibe el 2.
        assert_eq!(r.turn_seat(), Some(1));
        assert_eq!(r.seats[0].active, 1);
        assert_eq!(
            faces(&r.view(&rules, PLENTY).seats[0].hands[1].cards),
            [Eight, Two]
        );
        assert_eq!(r.seats[0].staked_units(), 2);
        r.act(1, Action::Stand, &mut shoe, &rules, PLENTY).unwrap();
        assert_eq!(r.turn_seat(), Some(2));
        r.act(2, Action::Stand, &mut shoe, &rules, PLENTY).unwrap();
        // Dealer 16 + 7 = 23: se pasa; ganan las tres manos.
        assert_eq!(r.phase, Phase::Settled);
        assert!(r
            .seats
            .iter()
            .flat_map(|s| &s.hands)
            .all(|h| h.outcome == Some(HandOutcome::Win)));
        assert_eq!(
            (
                r.seats[0].net_chips(&r.dealer),
                r.seats[1].net_chips(&r.dealer)
            ),
            (20, 10)
        );
    }

    #[test]
    fn allowed_depends_on_the_seat_in_turn_and_its_stack() {
        let rules = vegas();
        let (r, _) = deal(&[Eight, Ten, Six, Eight, Nine, Ten], &[(1, 10), (2, 10)]);
        assert!(r.allowed(&rules, PLENTY).split);
        assert!(
            !r.allowed(&rules, 0).split && !r.allowed(&rules, 0).double,
            "sin fichas extra"
        );
    }

    #[test]
    fn each_seat_decides_insurance_before_the_peek() {
        let rules = vegas();
        // s1: 10,7 (asegura); s2: 9,8 (no). Dealer As + K: blackjack.
        let (mut r, mut shoe) = deal(&[Ten, Nine, Ace, Seven, Eight, K], &[(1, 20), (2, 20)]);
        assert_eq!(r.pending_insurance(), [1, 2]);
        assert_eq!(r.turn_seat(), None, "nadie juega hasta decidir el seguro");
        assert_eq!(
            r.insure(1, true, 9, &mut shoe, &rules),
            Err(GameError::ActionNotAllowed),
            "el seguro de 20 cuesta 10"
        );
        r.insure(1, true, 10, &mut shoe, &rules).unwrap();
        assert_eq!(
            r.insure(1, false, 10, &mut shoe, &rules),
            Err(GameError::ActionNotAllowed),
            "ya decidió"
        );
        assert_eq!(r.pending_insurance(), [2]);
        assert_eq!(r.phase, Phase::Insurance);
        r.insure(2, false, 100, &mut shoe, &rules).unwrap();
        assert!(r.peek_blackjack);
        let v = r.view(&rules, PLENTY);
        assert_eq!(
            (v.seats[0].insured, v.seats[1].insured),
            (Some(true), Some(false))
        );
        // Asegurado: −20 + 20. Sin seguro: −20.
        assert_eq!((v.seats[0].net, v.seats[1].net), (Some(0), Some(-20)));
    }
}
