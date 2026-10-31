//! La caché entre llamadas no cambia ningún resultado: con la caché fría, caliente o
//! apagada, `analyze_*` responde lo mismo bit a bit.

use engine::{
    analyze_dealer, analyze_hand, analyze_round, cache, Allowed, AnalyzeDealerRequest,
    AnalyzeHandRequest, AnalyzeRoundRequest, Rank, Rules, Shoe,
};

fn r(i: u8) -> Rank {
    Rank::new(i).unwrap()
}

fn request(decks: u8, hand: &[Rank], up: Rank) -> AnalyzeHandRequest {
    let mut shoe = Shoe::full(decks);
    for &c in hand.iter().chain([&up]) {
        shoe.remove(c);
    }
    AnalyzeHandRequest {
        version: 1,
        rules: Rules {
            decks,
            ..Rules::default()
        },
        shoe: *shoe.counts(),
        hand: hand.to_vec(),
        dealer_up: up,
        split_hands_so_far: 0,
        is_split_aces: false,
        allowed: Allowed::default(),
    }
}

/// Casos variados: manos duras, blandas, pares (con el split exacto y el aproximado) y
/// distintos mazos y cartas visibles.
fn cases() -> Vec<AnalyzeHandRequest> {
    let mut out = Vec::new();
    for decks in [1u8, 2, 6, 8] {
        for (hand, up) in [
            (vec![r(9), r(5)], r(9)), // 10,6 vs 10
            (vec![r(1), r(2)], r(1)), // 2,3 vs 2
            (vec![r(0), r(5)], r(4)), // A,6 vs 5
            (vec![r(7), r(7)], r(5)), // 8,8 vs 6
            (vec![r(2), r(2)], r(0)), // 3,3 vs As
            (vec![r(0), r(0)], r(9)), // A,A vs 10
            (vec![r(9), r(0)], r(0)), // blackjack vs As
        ] {
            out.push(request(decks, &hand, up));
        }
    }
    out
}

#[test]
fn cached_results_are_identical_to_cold_ones() {
    cache::set_enabled(false);
    let cold: Vec<_> = cases()
        .into_iter()
        .map(|q| analyze_hand(q).unwrap())
        .collect();

    cache::set_enabled(true);
    // Primera pasada (caché que se va llenando) y segunda (todo en caché).
    for _ in 0..2 {
        let warm: Vec<_> = cases()
            .into_iter()
            .map(|q| analyze_hand(q).unwrap())
            .collect();
        assert_eq!(warm, cold);
    }
    let (solvers, nodes) = cache::stats();
    assert!(solvers <= cache::SLOTS);
    // El tope se aplica antes de cada cálculo: después puede pasarse por lo que agregó
    // la última llamada (como mucho ~345 mil nodos).
    assert!(nodes <= cache::MAX_NODES + 400_000, "{nodes}");
    cache::clear();
}

#[test]
fn the_next_decision_reuses_the_previous_one() {
    // 10,2 vs 10; después de pedir un 3: 10,2,3 con el zapato sin ese 3.
    let first = request(6, &[r(9), r(1)], r(9));
    let mut next = first.clone();
    next.hand.push(r(2));
    next.shoe[2] -= 1;
    next.allowed = Allowed {
        double: false,
        split: false,
        surrender: false,
    };

    cache::set_enabled(false);
    let cold = analyze_hand(next.clone()).unwrap();
    cache::set_enabled(true);
    analyze_hand(first).unwrap();
    let (_, before) = cache::stats();
    let warm = analyze_hand(next).unwrap();
    let (_, after) = cache::stats();
    assert_eq!(warm, cold);
    // Todo lo de la segunda decisión ya estaba memoizado: no hizo falta casi nada nuevo.
    assert!(after - before < before / 20, "{before} → {after}");
    cache::clear();
}

#[test]
fn dealer_and_round_are_identical_too() {
    let rules = Rules::default();
    let mut shoe = Shoe::full(6);
    for c in [r(9), r(5), r(8), r(1), r(5)] {
        shoe.remove(c);
    }
    let round = AnalyzeRoundRequest {
        version: 1,
        rules: rules.clone(),
        shoe: *shoe.counts(),
        hands: vec![vec![r(9), r(5)], vec![r(8), r(1)]],
        up: r(5),
    };
    let dealer = AnalyzeDealerRequest {
        version: 1,
        rules,
        shoe: *shoe.counts(),
        up: r(5),
        hole: None,
        drawn: vec![],
    };
    cache::set_enabled(false);
    let (cold_round, cold_dealer) = (
        analyze_round(round.clone()).unwrap(),
        analyze_dealer(dealer.clone()).unwrap(),
    );
    cache::set_enabled(true);
    for _ in 0..2 {
        assert_eq!(analyze_round(round.clone()).unwrap(), cold_round);
        assert_eq!(analyze_dealer(dealer.clone()).unwrap(), cold_dealer);
    }
    cache::clear();
}

/// Cuántos nodos deja el peor caso en la caché (para calibrar `MAX_NODES`).
#[test]
fn worst_case_node_counts() {
    cache::set_enabled(true);
    for (decks, hand, up) in [
        (8u8, vec![r(1), r(2)], r(1)),
        (8, vec![r(2), r(2)], r(5)),
        (8, vec![r(7), r(7)], r(9)),
        (1, vec![r(2), r(2)], r(5)),
    ] {
        cache::clear();
        analyze_hand(request(decks, &hand, up)).unwrap();
        eprintln!("{decks} mazos {hand:?} vs {up}: {} nodos", cache::stats().1);
    }
    cache::clear();
}
