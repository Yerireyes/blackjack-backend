//! Propiedades con zapatos, reglas y manos aleatorias válidas.

use engine::{
    analyze_dealer, analyze_hand, Action, Allowed, AnalyzeDealerRequest, AnalyzeHandRequest, Rank,
    Rules, Shoe, Surrender,
};
use proptest::prelude::*;

const EPS: f64 = 1e-9;

/// Saca una carta al azar (ponderada por conteo) usando `pick` en [0, 1).
fn draw(shoe: &mut Shoe, pick: f64) -> Rank {
    let mut target = (pick * shoe.total() as f64) as u16;
    for r in Rank::all() {
        if target < shoe.count(r) {
            shoe.remove(r);
            return r;
        }
        target -= shoe.count(r);
    }
    unreachable!()
}

#[derive(Debug, Clone)]
struct State {
    rules: Rules,
    shoe: [u16; 10],
    hand: Vec<Rank>,
    up: Rank,
}

fn state() -> impl Strategy<Value = State> {
    (
        1u8..=8,
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        prop::collection::vec(0.0f64..1.0, 2..5),
        0.0f64..1.0,
        prop::collection::vec(0.0f64..1.0, 0..60),
    )
        .prop_map(|(decks, h17, peek, late, hand_picks, up_pick, burn)| {
            let rules = Rules {
                decks,
                dealer_hits_soft17: h17,
                dealer_peeks: peek,
                surrender: if late {
                    Surrender::Late
                } else {
                    Surrender::None
                },
                ..Rules::default()
            };
            let mut shoe = Shoe::full(decks);
            // Cartas ya jugadas en rondas anteriores (a lo sumo un tercio del zapato).
            let max_burn = shoe.total() as usize / 3;
            for &p in burn.iter().take(max_burn) {
                draw(&mut shoe, p);
            }
            let up = draw(&mut shoe, up_pick);
            let mut hand = Vec::new();
            for &p in &hand_picks {
                let c = draw(&mut shoe, p);
                let total: u8 = hand.iter().chain([&c]).map(|r: &Rank| r.value()).sum();
                if hand.len() >= 2 && total > 21 {
                    shoe.add(c);
                    break;
                }
                hand.push(c);
            }
            State {
                rules,
                shoe: *shoe.counts(),
                hand,
                up,
            }
        })
        .prop_filter("mano válida y zapato suficiente", |s| {
            let total: u8 = s.hand.iter().map(|r| r.value()).sum();
            total <= 21 && Shoe::new(s.shoe).value_sum() >= engine::MIN_SHOE_VALUE
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn hand_invariants(s in state()) {
        let req = AnalyzeHandRequest {
            version: 7,
            rules: s.rules.clone(),
            shoe: s.shoe,
            hand: s.hand.clone(),
            dealer_up: s.up,
            split_hands_so_far: 0,
            is_split_aces: false,
            allowed: Allowed::default(),
        };
        let res = analyze_hand(req).unwrap();
        prop_assert_eq!(res.version, 7);

        let d = res.dealer;
        let dsum = d.p17 + d.p18 + d.p19 + d.p20 + d.p21 + d.p_blackjack + d.p_bust;
        prop_assert!((dsum - 1.0).abs() < EPS, "dealer suma {}", dsum);
        if s.rules.dealer_peeks && (s.up == Rank::ACE || s.up == Rank::TEN) {
            prop_assert_eq!(d.p_blackjack, 0.0);
        }
        prop_assert!((0.0..=1.0).contains(&res.p_bust_on_hit));

        let best_ev = res.actions.iter().map(|a| a.ev).fold(f64::NEG_INFINITY, f64::max);
        let best = res.actions.iter().find(|a| a.action == res.best).unwrap();
        prop_assert_eq!(best.ev, best_ev);

        for a in &res.actions {
            let sum = a.p_win + a.p_push + a.p_lose;
            prop_assert!((sum - 1.0).abs() < EPS, "{:?} suma {}", a.action, sum);
            prop_assert!(a.p_win >= -EPS && a.p_push >= -EPS && a.p_lose >= -EPS);
            match a.action {
                Action::Stand => prop_assert!(a.ev >= -1.0 - EPS && a.ev <= s.rules.blackjack_pays + EPS),
                Action::Hit => prop_assert!(a.ev.abs() <= 1.0 + EPS),
                Action::Double | Action::Split => prop_assert!(a.ev.abs() <= 2.0 + EPS),
                Action::Surrender => prop_assert_eq!(a.ev, -0.5),
            }
            if a.action != Action::Split {
                prop_assert!(a.exact);
            }
        }
    }

    #[test]
    fn dealer_known_hole_sums_to_one(s in state(), hole_pick in 0.0f64..1.0) {
        let mut shoe = Shoe::new(s.shoe);
        let hole = draw(&mut shoe, hole_pick);
        prop_assume!(shoe.value_sum() >= engine::MIN_SHOE_VALUE);
        let res = analyze_dealer(AnalyzeDealerRequest {
            version: 1, rules: s.rules, shoe: *shoe.counts(), up: s.up, hole: Some(hole), drawn: Vec::new(),
        }).unwrap();
        let d = res.dealer;
        let dsum = d.p17 + d.p18 + d.p19 + d.p20 + d.p21 + d.p_blackjack + d.p_bust;
        prop_assert!((dsum - 1.0).abs() < EPS);
    }
}
