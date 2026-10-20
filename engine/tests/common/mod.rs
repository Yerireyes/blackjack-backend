#![allow(dead_code)]

use engine::{Allowed, AnalyzeHandRequest, Rank, Rules, Shoe};

pub fn r(i: u8) -> Rank {
    Rank::new(i).unwrap()
}

/// Zapato completo de `decks` mazos sin las cartas visibles.
pub fn shoe_minus(decks: u8, visible: &[Rank]) -> [u16; 10] {
    let mut s = Shoe::full(decks);
    for &c in visible {
        s.remove(c);
    }
    *s.counts()
}

pub fn hand_request(rules: &Rules, hand: &[Rank], up: Rank) -> AnalyzeHandRequest {
    let mut visible = hand.to_vec();
    visible.push(up);
    AnalyzeHandRequest {
        version: 1,
        rules: rules.clone(),
        shoe: shoe_minus(rules.decks, &visible),
        hand: hand.to_vec(),
        dealer_up: up,
        split_hands_so_far: 0,
        is_split_aces: false,
        allowed: Allowed::default(),
    }
}
