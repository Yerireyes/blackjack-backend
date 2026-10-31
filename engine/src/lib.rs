//! Motor exacto de probabilidades de blackjack.
//!
//! Toda probabilidad sale del vector de conteos del zapato recibido, sacando
//! cartas sin reposición y memoizando por la clave del zapato. No hay tablas de
//! estrategia ni probabilidades precalculadas. El motor no depende del entorno
//! (ni red, ni archivos, ni reloj).

pub mod dealer;
pub mod dto;
pub mod error;
pub mod game;
pub mod hand;
pub mod hole;
pub mod outcome;
pub mod player;
pub mod rank;
pub mod rules;
pub mod shoe;

pub use dealer::{DealerDist, DealerSolver};
pub use dto::*;
pub use error::EngineError;
pub use hand::{Hand, HandState};
pub use outcome::Outcome;
pub use player::{Decision, Solver};
pub use rank::Rank;
pub use rules::{DoubleOn, Rules, Surrender};
pub use shoe::Shoe;

use rank::NUM_RANKS;

/// Suma mínima de valores (As = 1) que debe tener el zapato. Una mano del jugador
/// consume como mucho 31 puntos y la del dealer 26, así que con 60 la recursión
/// nunca se queda sin cartas.
pub const MIN_SHOE_VALUE: u32 = 60;

pub fn analyze_hand(req: AnalyzeHandRequest) -> Result<AnalyzeHandResponse, EngineError> {
    req.rules.validate()?;
    let mut visible = req.hand.clone();
    visible.push(req.dealer_up);
    let shoe = validate_shoe(&req.rules, &req.shoe, &visible, Some(req.dealer_up))?;
    validate_hand(&req.hand)?;
    if req.is_split_aces && req.split_hands_so_far == 0 {
        return Err(EngineError::InvalidHand(
            "isSplitAces requiere splitHandsSoFar > 0".into(),
        ));
    }
    let mut solver = Solver::new(&req.rules, req.dealer_up);
    let mut res = hand_response(
        &mut solver,
        &shoe,
        &req.hand,
        req.split_hands_so_far,
        req.is_split_aces,
        req.allowed,
    );
    res.version = req.version;
    Ok(res)
}

pub fn analyze_dealer(req: AnalyzeDealerRequest) -> Result<AnalyzeDealerResponse, EngineError> {
    req.rules.validate()?;
    if !req.drawn.is_empty() && req.hole.is_none() {
        return Err(EngineError::InvalidHand(
            "las cartas sacadas por el dealer (drawn) requieren conocer la oculta (hole)".into(),
        ));
    }
    let mut cards = vec![req.up];
    cards.extend(req.hole);
    cards.extend(req.drawn.iter().copied());
    if !req.drawn.is_empty() {
        let before_last = Hand::from_cards(&cards[..cards.len() - 1]);
        if before_last.is_bust() || before_last.is_two_card_21() {
            return Err(EngineError::InvalidHand(
                "el dealer no podía sacar más cartas (se había pasado o tenía blackjack)".into(),
            ));
        }
    }
    let peek_up = if req.hole.is_some() {
        None
    } else {
        Some(req.up)
    };
    let shoe = validate_shoe(&req.rules, &req.shoe, &cards, peek_up)?;
    let mut solver = Solver::new(&req.rules, req.up);
    let (dist, p_bust_next) = match req.hole {
        Some(_) => {
            let dealer = solver.dealer_mut();
            (
                dealer.with_cards(&cards, &shoe),
                dealer.p_bust_next(&cards, &shoe),
            )
        }
        None => (solver.dealer_dist(&shoe), None),
    };
    Ok(AnalyzeDealerResponse {
        version: req.version,
        dealer: dist.into(),
        p_bust_next,
        compute_ms: 0.0,
    })
}

pub fn analyze_round(req: AnalyzeRoundRequest) -> Result<AnalyzeRoundResponse, EngineError> {
    req.rules.validate()?;
    if req.hands.is_empty() {
        return Err(EngineError::InvalidHand("la ronda no tiene manos".into()));
    }
    let mut visible: Vec<Rank> = req.hands.iter().flatten().copied().collect();
    visible.push(req.up);
    let shoe = validate_shoe(&req.rules, &req.shoe, &visible, Some(req.up))?;
    for h in &req.hands {
        validate_hand(h)?;
    }
    // Mismo zapato y misma carta visible: un solo Solver comparte toda la memoización.
    let mut solver = Solver::new(&req.rules, req.up);
    let hands: Vec<AnalyzeHandResponse> = req
        .hands
        .iter()
        .map(|cards| {
            let mut r = hand_response(&mut solver, &shoe, cards, 0, false, Allowed::default());
            r.version = req.version;
            r
        })
        .collect();
    let best_sum: f64 = hands
        .iter()
        .map(|h| {
            h.actions
                .iter()
                .find(|a| a.action == h.best)
                .map_or(0.0, |a| a.ev)
        })
        .sum();
    Ok(AnalyzeRoundResponse {
        version: req.version,
        house_edge_round: -best_sum / hands.len() as f64,
        hands,
        compute_ms: 0.0,
    })
}

/// Calcula todas las acciones disponibles para una mano ya validada.
fn hand_response(
    solver: &mut Solver,
    shoe: &Shoe,
    cards: &[Rank],
    split_hands_so_far: u8,
    is_split_aces: bool,
    allowed: Allowed,
) -> AnalyzeHandResponse {
    let rules = solver.rules().clone();
    let from_split = split_hands_so_far > 0;
    let state = HandState::new(cards, from_split);
    let hand = Hand::from_cards(cards);
    let result = |action, o: Outcome, exact| ActionResult {
        action,
        ev: o.ev,
        p_win: o.win,
        p_push: o.push,
        p_lose: o.lose,
        exact,
    };

    let mut actions = Vec::new();
    if state.is_natural() {
        actions.push(result(Action::Stand, solver.natural(shoe), true));
    } else {
        let locked_aces = is_split_aces && !rules.hit_split_aces;
        actions.push(result(Action::Stand, solver.stand(shoe, hand), true));
        if !locked_aces {
            actions.push(result(Action::Hit, solver.hit(shoe, hand), true));
        }
        let can_double = allowed.double
            && !locked_aces
            && state.n_cards == 2
            && rules.can_double_total(state.total, state.soft)
            && (!from_split || rules.double_after_split);
        if can_double {
            actions.push(result(Action::Double, solver.double(shoe, hand), true));
        }
        if let Some(pair) = state.is_pair {
            let hands_after = split_hands_so_far as u16 + 2;
            let aces_ok = !(pair.is_ace() && is_split_aces) || rules.resplit_aces;
            if allowed.split && hands_after <= rules.max_split_hands as u16 && aces_ok {
                // Manos que pueden salir de este par: tope de la regla menos las
                // que ya hay aparte de esta; 2 si no se puede re-dividir.
                let can_resplit = !pair.is_ace() || rules.resplit_aces;
                let max_hands = if can_resplit {
                    rules.max_split_hands - split_hands_so_far
                } else {
                    2
                };
                // Exacto solo con dos manos, sin otras manos divididas pendientes y
                // si el cálculo exacto cupo en el presupuesto.
                let (o, two_exact) = solver.split(shoe, pair, max_hands);
                let exact = two_exact && max_hands == 2 && !from_split;
                actions.push(result(Action::Split, o, exact));
            }
        }
        let can_surrender = allowed.surrender
            && rules.surrender == Surrender::Late
            && rules.dealer_peeks
            && state.n_cards == 2
            && !from_split;
        if can_surrender {
            actions.push(result(Action::Surrender, player::surrender(), true));
        }
    }

    // Mayor EV; en empate exacto gana la primera (plantarse va primero).
    let best = actions
        .iter()
        .fold(None::<&ActionResult>, |b, a| match b {
            Some(b) if b.ev >= a.ev => Some(b),
            _ => Some(a),
        })
        .map(|a| a.action)
        .unwrap_or(Action::Stand);

    AnalyzeHandResponse {
        version: 0,
        actions,
        best,
        p_bust_on_hit: solver.p_bust_on_hit(shoe, hand),
        dealer: solver.dealer_dist(shoe).into(),
        compute_ms: 0.0,
    }
}

/// Valida el zapato frente a las cartas visibles y devuelve el `Shoe`.
/// `peek_up`: carta visible del dealer cuando la oculta sigue dentro del zapato.
fn validate_shoe(
    rules: &Rules,
    counts: &[u16; NUM_RANKS],
    visible: &[Rank],
    peek_up: Option<Rank>,
) -> Result<Shoe, EngineError> {
    let shoe = Shoe::new(*counts);
    let mut seen = [0u16; NUM_RANKS];
    for r in visible {
        seen[r.index()] += 1;
    }
    for r in Rank::all() {
        let cap = Shoe::capacity(rules.decks, r);
        if shoe.count(r) > cap {
            return Err(EngineError::InvalidShoe(format!(
                "hay {} cartas de rango {r} y {} mazo(s) solo tienen {cap}",
                shoe.count(r),
                rules.decks
            )));
        }
        if shoe.count(r) + seen[r.index()] > cap {
            return Err(EngineError::InvalidShoe(format!(
                "la carta {r} de la mesa no puede estar en ese zapato: ya quedan {} y {} mazo(s) solo tienen {cap}",
                shoe.count(r),
                rules.decks
            )));
        }
    }
    if shoe.value_sum() < MIN_SHOE_VALUE {
        return Err(EngineError::InvalidShoe(format!(
            "quedan muy pocas cartas para calcular (suma de valores {} < {MIN_SHOE_VALUE}); hay que rebarajar",
            shoe.value_sum()
        )));
    }
    if let Some(up) = peek_up {
        if let Some(f) = hole::forbidden_rank(up, rules) {
            if shoe.count(f) == shoe.total() {
                return Err(EngineError::InvalidShoe(format!(
                    "todas las cartas restantes son {f}: el dealer tendría blackjack tras el peek"
                )));
            }
        }
    }
    Ok(shoe)
}

fn validate_hand(cards: &[Rank]) -> Result<(), EngineError> {
    if cards.len() < 2 {
        return Err(EngineError::InvalidHand(
            "la mano necesita al menos 2 cartas".into(),
        ));
    }
    if Hand::from_cards(cards).is_bust() {
        return Err(EngineError::InvalidHand("la mano ya se pasó de 21".into()));
    }
    Ok(())
}
