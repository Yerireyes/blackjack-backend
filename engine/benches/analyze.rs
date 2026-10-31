use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion};
use engine::{analyze_hand, Allowed, AnalyzeHandRequest, Rank, Rules, Shoe};

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

fn bench(c: &mut Criterion) {
    let r = |i| Rank::new(i).unwrap();
    // En frío, como cada llamada antes de la caché: los tiempos siguen siendo comparables.
    engine::cache::set_enabled(false);
    for decks in [1u8, 2, 6, 8] {
        // Mano dura baja: la recursión de pedir más profunda.
        let req = request(decks, &[r(1), r(2)], r(1));
        c.bench_function(&format!("hand_5_vs_2_{decks}d"), |b| {
            b.iter(|| analyze_hand(black_box(req.clone())).unwrap())
        });
        let req = request(decks, &[r(1), r(1)], r(5));
        c.bench_function(&format!("split_3_3_vs_6_{decks}d"), |b| {
            b.iter(|| analyze_hand(black_box(req.clone())).unwrap())
        });
    }

    // Con la caché: la decisión siguiente a pedir (10,2 vs 10 → 10,2,3), después de haber
    // analizado la primera, como en una mano del front (las repeticiones también salen
    // de la caché).
    engine::cache::set_enabled(true);
    let first = request(6, &[r(9), r(1)], r(9));
    let mut next = first.clone();
    next.hand.push(r(2));
    next.shoe[2] -= 1;
    analyze_hand(first).unwrap();
    c.bench_function("cached_next_decision_6d", |b| {
        b.iter(|| analyze_hand(black_box(next.clone())).unwrap())
    });
    engine::cache::set_enabled(false);
    c.bench_function("cold_next_decision_6d", |b| {
        b.iter(|| analyze_hand(black_box(next.clone())).unwrap())
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10);
    targets = bench
}
criterion_main!(benches);
