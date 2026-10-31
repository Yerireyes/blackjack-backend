mod common;

use common::{hand_request, r, shoe_minus};
use engine::{analyze_dealer, analyze_hand, Action, AnalyzeDealerRequest, Rank, Rules};

fn bust_with_up(rules: &Rules, shoe: [u16; 10], up: Rank) -> f64 {
    analyze_dealer(AnalyzeDealerRequest {
        version: 1,
        rules: rules.clone(),
        shoe,
        up,
        hole: None,
        drawn: Vec::new(),
    })
    .unwrap()
    .dealer
    .p_bust
}

#[test]
fn removing_tens_lowers_dealer_bust_with_six() {
    let rules = Rules {
        decks: 6,
        ..Rules::default()
    };
    let six = r(5);
    let full = shoe_minus(6, &[six]);
    let mut no_tens = full;
    no_tens[9] = 0;
    let before = bust_with_up(&rules, full, six);
    let after = bust_with_up(&rules, no_tens, six);
    assert!(after < before, "sin dieces: {after} debería ser < {before}");
}

#[test]
fn hard_16_cannot_bust_without_six_to_ten() {
    let rules = Rules {
        decks: 2,
        ..Rules::default()
    };
    let hand = [Rank::TEN, r(5)];
    let up = r(4);
    let mut req = hand_request(&rules, &hand, up);
    for i in 5..10 {
        req.shoe[i] = 0;
    }
    let res = analyze_hand(req).unwrap();
    assert_eq!(res.p_bust_on_hit, 0.0);
    // Sin riesgo de pasarse, pedir domina a plantarse con 16.
    let ev = |a| res.actions.iter().find(|x| x.action == a).unwrap().ev;
    assert!(ev(Action::Hit) > ev(Action::Stand));
}

#[test]
fn card_removal_effect_between_one_and_eight_decks() {
    let hand = [Rank::TEN, r(1)];
    let up = r(3);
    let one = analyze_hand(hand_request(
        &Rules {
            decks: 1,
            ..Rules::default()
        },
        &hand,
        up,
    ))
    .unwrap();
    let eight = analyze_hand(hand_request(
        &Rules {
            decks: 8,
            ..Rules::default()
        },
        &hand,
        up,
    ))
    .unwrap();
    for (a, b) in one.actions.iter().zip(eight.actions.iter()) {
        assert_eq!(a.action, b.action);
        assert_ne!(
            a.ev, b.ev,
            "{:?} debería diferir entre 1 y 8 mazos",
            a.action
        );
    }
}

#[test]
fn deterministic_bit_for_bit() {
    let rules = Rules {
        decks: 6,
        dealer_hits_soft17: true,
        ..Rules::default()
    };
    let req = hand_request(&rules, &[r(7), r(7)], Rank::TEN);
    let a = analyze_hand(req.clone()).unwrap();
    let b = analyze_hand(req).unwrap();
    assert_eq!(a.actions.len(), b.actions.len());
    for (x, y) in a.actions.iter().zip(b.actions.iter()) {
        assert_eq!(x.ev.to_bits(), y.ev.to_bits());
        assert_eq!(x.p_win.to_bits(), y.p_win.to_bits());
        assert_eq!(x.p_push.to_bits(), y.p_push.to_bits());
        assert_eq!(x.p_lose.to_bits(), y.p_lose.to_bits());
    }
    assert_eq!(a.dealer, b.dealer);
}

/// Estrategia básica conocida (6 mazos, S17, DAS): decisiones clásicas que el
/// motor debe reproducir a partir de la composición, sin tablas.
#[test]
fn reproduces_classic_basic_strategy_decisions() {
    let rules = Rules {
        decks: 6,
        ..Rules::default()
    };
    let cases = [
        (vec![Rank::TEN, r(5)], Rank::TEN, Action::Hit), // 16 vs 10
        (vec![Rank::TEN, r(5)], r(5), Action::Stand),    // 16 vs 6
        (vec![r(4), r(5)], r(4), Action::Double),        // 11 vs 5
        (vec![Rank::TEN, r(1)], r(1), Action::Hit),      // 12 vs 3
        (vec![r(7), r(7)], Rank::TEN, Action::Split),    // 8,8 vs 10
        (vec![Rank::ACE, Rank::ACE], r(5), Action::Split), // A,A vs 6
        (vec![Rank::TEN, Rank::TEN], r(5), Action::Stand), // 20 vs 6
        (vec![Rank::ACE, r(6)], r(2), Action::Double),   // A,7 vs 3... S17: doblar
        (vec![Rank::ACE, r(6)], r(8), Action::Hit),      // A,7 vs 9
    ];
    for (hand, up, want) in cases {
        let res = analyze_hand(hand_request(&rules, &hand, up)).unwrap();
        assert_eq!(res.best, want, "{hand:?} vs {up}: {:?}", res.actions);
    }
}

#[test]
fn natural_pays_three_to_two_after_peek() {
    let rules = Rules {
        decks: 6,
        ..Rules::default()
    };
    let res = analyze_hand(hand_request(&rules, &[Rank::ACE, Rank::TEN], Rank::TEN)).unwrap();
    assert_eq!(res.actions.len(), 1);
    assert!((res.actions[0].ev - 1.5).abs() < 1e-12);
}

#[test]
fn rejects_impossible_input() {
    let rules = Rules {
        decks: 1,
        ..Rules::default()
    };
    // Cinco ases en un mazo: cuatro en el zapato y uno en la mano.
    let mut req = hand_request(&rules, &[Rank::ACE, r(5)], r(4));
    req.shoe[0] = 4;
    assert!(analyze_hand(req).is_err());
    // Mano pasada.
    let req = hand_request(&rules, &[Rank::TEN, Rank::TEN, r(4)], r(4));
    assert!(analyze_hand(req).is_err());
    // Zapato casi vacío.
    let mut req = hand_request(&rules, &[Rank::TEN, r(4)], r(4));
    req.shoe = [0, 0, 0, 0, 0, 0, 0, 0, 0, 3];
    assert!(analyze_hand(req).is_err());
}

#[test]
fn split_is_exact_only_with_two_hands() {
    let pair = [r(7), r(7)];
    let split = |rules: &Rules| {
        let res = analyze_hand(hand_request(rules, &pair, Rank::TEN)).unwrap();
        res.actions
            .iter()
            .find(|a| a.action == Action::Split)
            .unwrap()
            .clone()
    };
    let two = split(&Rules {
        decks: 1,
        max_split_hands: 2,
        ..Rules::default()
    });
    let four = split(&Rules {
        decks: 1,
        max_split_hands: 4,
        ..Rules::default()
    });
    assert!(two.exact);
    assert!(!four.exact);
    // Poder re-dividir 8,8 contra 10 mejora el EV.
    assert!(four.ev > two.ev);
    for a in [two, four] {
        assert!((a.p_win + a.p_push + a.p_lose - 1.0).abs() < 1e-9);
    }

    // Ases sin re-división: siempre dos manos, exacto aunque el tope sea 4.
    let res = analyze_hand(hand_request(
        &Rules {
            decks: 6,
            ..Rules::default()
        },
        &[Rank::ACE, Rank::ACE],
        r(5),
    ))
    .unwrap();
    assert!(
        res.actions
            .iter()
            .find(|a| a.action == Action::Split)
            .unwrap()
            .exact
    );
}

#[test]
fn split_from_split_hand_is_never_exact() {
    let rules = Rules {
        decks: 6,
        max_split_hands: 4,
        ..Rules::default()
    };
    let mut req = hand_request(&rules, &[r(7), r(7)], r(5));
    req.split_hands_so_far = 1;
    let res = analyze_hand(req.clone()).unwrap();
    assert!(
        !res.actions
            .iter()
            .find(|a| a.action == Action::Split)
            .unwrap()
            .exact
    );
    // Con el tope alcanzado ya no se ofrece dividir.
    req.split_hands_so_far = 3;
    let res = analyze_hand(req).unwrap();
    assert!(res.actions.iter().all(|a| a.action != Action::Split));
}

fn dealer_with_cards(
    rules: &Rules,
    up: Rank,
    hole: Rank,
    drawn: &[Rank],
) -> engine::AnalyzeDealerResponse {
    let mut visible = vec![up, hole];
    visible.extend_from_slice(drawn);
    analyze_dealer(AnalyzeDealerRequest {
        version: 1,
        rules: rules.clone(),
        shoe: shoe_minus(rules.decks, &visible),
        up,
        hole: Some(hole),
        drawn: drawn.to_vec(),
    })
    .unwrap()
}

#[test]
fn dealer_drawn_cards_condition_distribution_and_next_bust() {
    let rules = Rules {
        decks: 1,
        ..Rules::default()
    };
    // 10 + 2 + 4 = 16: debe pedir; se pasa con 6, 7, 8, 9 o 10.
    let res = dealer_with_cards(&rules, Rank::TEN, r(1), &[r(3)]);
    let shoe = shoe_minus(1, &[Rank::TEN, r(1), r(3)]);
    let n: u16 = shoe.iter().sum();
    let bust: u16 = shoe[5..].iter().sum();
    let expected = bust as f64 / n as f64;
    assert!((res.p_bust_next.unwrap() - expected).abs() < 1e-15);
    let d = res.dealer;
    let sum = d.p17 + d.p18 + d.p19 + d.p20 + d.p21 + d.p_blackjack + d.p_bust;
    assert!((sum - 1.0).abs() < 1e-12);
    assert_eq!(d.p_blackjack, 0.0);

    // Sin cartas sacadas coincide con la distribución conociendo solo la oculta.
    let two = dealer_with_cards(&rules, Rank::TEN, r(5), &[]);
    assert!(two.p_bust_next.is_some());
}

#[test]
fn dealer_standing_or_bust_has_no_next_bust() {
    let rules = Rules {
        decks: 6,
        ..Rules::default()
    };
    // 10 + 8 = 18: se planta.
    let stand = dealer_with_cards(&rules, Rank::TEN, r(7), &[]);
    assert_eq!(stand.p_bust_next, None);
    assert_eq!(stand.dealer.p18, 1.0);
    // 10 + 6 + 10: se pasó.
    let bust = dealer_with_cards(&rules, Rank::TEN, r(5), &[Rank::TEN]);
    assert_eq!(bust.p_bust_next, None);
    assert_eq!(bust.dealer.p_bust, 1.0);
    // A + 6 con H17 pide; con S17 se planta.
    let h17 = Rules {
        dealer_hits_soft17: true,
        ..rules.clone()
    };
    assert!(dealer_with_cards(&h17, Rank::ACE, r(5), &[])
        .p_bust_next
        .is_some());
    assert_eq!(
        dealer_with_cards(&rules, Rank::ACE, r(5), &[]).p_bust_next,
        None
    );
}

#[test]
fn dealer_drawn_requires_hole_and_rejects_cards_after_bust() {
    let rules = Rules {
        decks: 6,
        ..Rules::default()
    };
    let no_hole = analyze_dealer(AnalyzeDealerRequest {
        version: 1,
        rules: rules.clone(),
        shoe: shoe_minus(6, &[Rank::TEN, r(4)]),
        up: Rank::TEN,
        hole: None,
        drawn: vec![r(4)],
    });
    assert!(no_hole.is_err());
    let after_bust = analyze_dealer(AnalyzeDealerRequest {
        version: 1,
        rules: rules.clone(),
        shoe: shoe_minus(6, &[Rank::TEN, r(5), Rank::TEN, r(1)]),
        up: Rank::TEN,
        hole: Some(r(5)),
        drawn: vec![Rank::TEN, r(1)],
    });
    assert!(after_bust.is_err());
}
