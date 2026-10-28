//! Casos de `tableStore.test.ts` del front portados, más propiedades: nunca se
//! reparte una carta dos veces, el neto es entero y la oculta no se ve antes de tiempo.

use super::card::Face::{self, *};
use super::*;
use crate::dto::Action;
use crate::rules::Rules;

fn vegas() -> Rules {
    Preset::Vegas.rules()
}

fn deal(rules: &Rules, top: &[Face]) -> (Round, GameShoe) {
    let mut shoe = GameShoe::stacked(rules.decks, top);
    let round = Round::deal(&mut shoe, rules).unwrap();
    (round, shoe)
}

fn faces(cards: &[CardView]) -> Vec<Face> {
    cards.iter().map(|c| c.rank).collect()
}

const PLENTY: u32 = 100;

#[test]
fn deals_player_up_player_hole_without_revealing_it() {
    let (round, shoe) = deal(&vegas(), &[Ten, Nine, Six, Seven]);
    let view = round.player_view(10);
    assert_eq!(faces(&view.hands[0].cards), [Ten, Six]);
    let dealer = round.dealer_view();
    assert_eq!(dealer.up.rank, Nine);
    assert_eq!(dealer.hole, None);
    assert_eq!(round.dealer.hole.face, Seven);
    // 312 − 3 visibles: la oculta sigue contando como no vista.
    assert_eq!(round.unseen_counts(&shoe).iter().sum::<u16>(), 312 - 3);
    assert_eq!(round.phase, RoundPhase::Player);
}

#[test]
fn peek_blackjack_ends_the_round_at_the_deal() {
    let rules = vegas();
    let (mut round, mut shoe) = deal(&rules, &[Ten, Ace, Seven, K]);
    assert!(round.peek_blackjack);
    assert_eq!(round.phase, RoundPhase::Settled);
    assert_eq!(round.hands[0].outcome, Some(HandOutcome::Lose));
    assert_eq!(round.net_chips(10), -10);
    assert_eq!(round.dealer_view().hole.map(|c| c.rank), Some(K));
    assert_eq!(round.unseen_counts(&shoe).iter().sum::<u16>(), 312 - 4);
    assert_eq!(
        round.act(Action::Hit, &mut shoe, &rules, PLENTY),
        Err(GameError::ActionNotAllowed)
    );
    assert_eq!(round.hands[0].cards.len(), 2);
}

#[test]
fn split_advances_and_each_hand_gets_its_second_card() {
    let rules = vegas();
    let (mut round, mut shoe) = deal(&rules, &[Eight, Six, Eight, Ten, Three, Five]);
    assert!(round.allowed(&rules, PLENTY).split);
    round.act(Action::Split, &mut shoe, &rules, PLENTY).unwrap();
    let view = round.player_view(10);
    assert_eq!(view.hands.len(), 2);
    assert_eq!(faces(&view.hands[0].cards), [Eight, Three]);
    assert_eq!(faces(&view.hands[1].cards), [Eight]);
    assert_eq!(view.active, 0);
    round.act(Action::Stand, &mut shoe, &rules, PLENTY).unwrap();
    assert_eq!(round.active, 1);
    assert_eq!(faces(&round.player_view(10).hands[1].cards), [Eight, Five]);
    assert!(round.hands.iter().all(|h| h.from_split));
}

#[test]
fn dealer_reveals_hits_by_the_rules_and_settles() {
    // Jugador 10,8; dealer 6 + 10 = 16 → pide un 5 → 21.
    let rules = vegas();
    let (mut round, mut shoe) = deal(&rules, &[Ten, Six, Eight, Ten, Five]);
    round.act(Action::Stand, &mut shoe, &rules, PLENTY).unwrap();
    assert_eq!(round.phase, RoundPhase::Settled);
    let dealer = round.dealer_view();
    assert_eq!(dealer.hole.map(|c| c.rank), Some(Ten));
    assert_eq!(faces(&dealer.hits), [Five]);
    assert_eq!(round.hands[0].outcome, Some(HandOutcome::Lose));
    assert_eq!(round.net_units(), -1.0);
    // La ronda liquida según la apuesta.
    assert_eq!(round.net_chips(50), -50);
    assert_eq!(round.allowed(&rules, PLENTY), AllowedActions::default());
}

#[test]
fn cannot_double_or_split_without_funds_for_another_unit() {
    let rules = vegas();
    let (mut round, mut shoe) = deal(&rules, &[Eight, Six, Eight, Ten]);
    let rich = round.allowed(&rules, 1);
    assert!(rich.split && rich.double);
    let broke = round.allowed(&rules, 0);
    assert!(!broke.split && !broke.double);
    assert!(broke.hit && broke.stand && broke.surrender);
    assert_eq!(
        round.act(Action::Double, &mut shoe, &rules, 0),
        Err(GameError::ActionNotAllowed)
    );
    assert!(!round.hands[0].doubled);
}

#[test]
fn player_blackjack_pays_by_the_rule() {
    let (round, _) = deal(&vegas(), &[Ace, Nine, K, Seven]);
    assert_eq!(round.phase, RoundPhase::Settled);
    assert!(!round.peek_blackjack);
    assert_eq!(round.hands[0].outcome, Some(HandOutcome::Blackjack));
    assert_eq!(round.hands[0].net, Some(1.5));
    assert_eq!(round.net_chips(10), 15);
    // El dealer no pide: no hay ninguna mano viva.
    assert!(round.dealer.hits.is_empty());

    let (round, _) = deal(&Preset::SixToFive.rules(), &[Ace, Nine, K, Seven]);
    assert_eq!(round.net_chips(10), 12);
}

#[test]
fn surrender_returns_half() {
    let rules = vegas();
    let (mut round, mut shoe) = deal(&rules, &[Ten, Ten, Six, Seven]);
    round
        .act(Action::Surrender, &mut shoe, &rules, PLENTY)
        .unwrap();
    assert_eq!(round.hands[0].outcome, Some(HandOutcome::Surrender));
    assert_eq!(round.net_chips(10), -5);
    // Rendirse no está en Europa (sin peek).
    let europe = Preset::Europe.rules();
    let (round, _) = deal(&europe, &[Ten, Ten, Six, Seven]);
    assert!(!round.allowed(&europe, PLENTY).surrender);
}

#[test]
fn split_aces_get_one_card_and_are_not_blackjack() {
    let rules = vegas();
    let (mut round, mut shoe) = deal(&rules, &[Ace, Six, Ace, Nine, Ten, K]);
    round.act(Action::Split, &mut shoe, &rules, PLENTY).unwrap();
    assert_eq!(round.phase, RoundPhase::Settled);
    for h in &round.hands {
        assert_eq!(h.cards.len(), 2);
        assert!(h.split_aces);
        assert_ne!(h.outcome, Some(HandOutcome::Blackjack));
    }
}

#[test]
fn doubling_takes_one_card_and_doubles_the_result() {
    // 6,5 → dobla y saca un 10 (21); dealer 9 + 7 = 16 → pide un 8 → 24, se pasa.
    let rules = vegas();
    let (mut round, mut shoe) = deal(&rules, &[Six, Nine, Five, Seven, Ten, Eight]);
    round
        .act(Action::Double, &mut shoe, &rules, PLENTY)
        .unwrap();
    assert_eq!(round.hands[0].cards.len(), 3);
    assert_eq!(round.hands[0].outcome, Some(HandOutcome::Win));
    assert_eq!(round.net_chips(20), 40);
    assert_eq!(round.staked_units(), 2);
}

#[test]
fn europe_dealer_blackjack_without_peek_takes_doubles() {
    // Sin peek el jugador juega; el blackjack del dealer se lleva lo doblado.
    let rules = Preset::Europe.rules();
    let (mut round, mut shoe) = deal(&rules, &[Six, Ace, Five, K, Nine]);
    assert_eq!(round.phase, RoundPhase::Player);
    round
        .act(Action::Double, &mut shoe, &rules, PLENTY)
        .unwrap();
    assert_eq!(round.hands[0].outcome, Some(HandOutcome::Lose));
    assert_eq!(round.net_units(), -2.0);
}

#[test]
fn settle_hand_cases() {
    use super::card::Suit;
    use super::round::settle_hand;
    let rules = Preset::Europe.rules();
    let cards = |f: &[Face]| -> Vec<Card> {
        f.iter()
            .map(|&face| Card {
                face,
                suit: Suit::S,
            })
            .collect()
    };
    let hand = |f: &[Face]| PlayerHand {
        cards: cards(f),
        from_split: false,
        split_aces: false,
        doubled: false,
        surrendered: false,
        done: true,
        outcome: None,
        net: None,
    };
    let doubled = PlayerHand {
        doubled: true,
        ..hand(&[Six, Five, Nine])
    };
    assert_eq!(
        settle_hand(&doubled, &cards(&[Ace, K]), &rules),
        (HandOutcome::Lose, -2.0)
    );
    assert_eq!(
        settle_hand(&hand(&[Ten, Eight]), &cards(&[Nine, Nine]), &rules).0,
        HandOutcome::Push
    );
    let surrendered = PlayerHand {
        surrendered: true,
        ..hand(&[Ten, Six])
    };
    assert_eq!(
        settle_hand(&surrendered, &cards(&[Nine, Nine]), &rules).1,
        -0.5
    );
    assert_eq!(
        settle_hand(&hand(&[Ten, Two]), &cards(&[Six, Ten, K]), &rules).0,
        HandOutcome::Win
    );
}

/// Juega muchas rondas eligiendo acciones al azar entre las permitidas.
#[test]
fn random_play_properties() {
    let mut seed = 42u64;
    let mut below = move |n: usize| {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((seed >> 33) as usize) % n
    };
    for preset in [Preset::Vegas, Preset::Europe, Preset::SixToFive] {
        let rules = preset.rules();
        let total = rules.decks as u16 * 52;
        let mut shoe = GameShoe::shuffled(rules.decks, &mut below);
        for _ in 0..3000 {
            if shoe.needs_shuffle(rules.penetration, rules.max_split_hands) {
                shoe.shuffle(&mut below);
            }
            let unseen_before = shoe.remaining_counts().iter().sum::<u16>();
            let mut round = Round::deal(&mut shoe, &rules).unwrap();
            while round.phase == RoundPhase::Player {
                // La oculta no se ve mientras el jugador decide.
                assert_eq!(round.dealer_view().hole, None);
                let unseen = round.unseen_counts(&shoe).iter().sum::<u16>();
                let visible: usize = round.hands.iter().map(|h| h.cards.len()).sum::<usize>() + 1;
                assert_eq!(unseen as usize + visible, unseen_before as usize);
                let allowed = round.allowed(&rules, 3);
                let options: Vec<Action> = [
                    Action::Hit,
                    Action::Stand,
                    Action::Double,
                    Action::Split,
                    Action::Surrender,
                ]
                .into_iter()
                .filter(|&a| allowed.get(a))
                .collect();
                assert!(!options.is_empty(), "sin acciones en fase de jugador");
                let action = options[below(options.len())];
                round.act(action, &mut shoe, &rules, 3).unwrap();
            }
            assert!(round.dealer_view().hole.is_some());
            // Neto entero: con apuestas múltiplos de 10 no hay redondeo.
            let units = round.net_units();
            assert_eq!((units * 10.0).fract(), 0.0, "{units}");
            let shown: i64 = round
                .player_view(10)
                .hands
                .iter()
                .map(|h| h.net.unwrap())
                .sum();
            assert_eq!(shown, round.net_chips(10));
            // Ninguna carta repartida dos veces: lo repartido sale del zapato.
            let dealt = (unseen_before - shoe.remaining_counts().iter().sum::<u16>()) as usize;
            let on_table = round.hands.iter().map(|h| h.cards.len()).sum::<usize>()
                + 2
                + round.dealer.hits.len();
            assert_eq!(dealt, on_table);
            assert!(shoe.remaining_counts().iter().sum::<u16>() <= total);
        }
    }
}
