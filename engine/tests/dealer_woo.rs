//! Distribución final del dealer contra las 16 tablas publicadas por Wizard of Odds
//! (1 a 8 mazos, S17 y H17, dealer ya revisó y no tiene blackjack).

use engine::{analyze_dealer, AnalyzeDealerRequest, Rank, Rules, Shoe};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Fixture {
    tables: Vec<Table>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Table {
    decks: u8,
    dealer_hits_soft17: bool,
    rows: Vec<Row>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Row {
    up: u8,
    p17: f64,
    p18: f64,
    p19: f64,
    p20: f64,
    p21: f64,
    p_bust: f64,
}

const TOL: f64 = 1e-6;

#[test]
fn matches_all_published_tables() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("fixtures/woo_dealer.json")).expect("fixture JSON");
    assert_eq!(fixture.tables.len(), 16);
    let mut checked = 0;
    for t in &fixture.tables {
        let rules = Rules {
            decks: t.decks,
            dealer_hits_soft17: t.dealer_hits_soft17,
            dealer_peeks: true,
            ..Rules::default()
        };
        for row in &t.rows {
            let up = Rank::new(row.up).unwrap();
            // Se asume que la tabla solo retira la carta visible del dealer.
            let shoe = Shoe::full(t.decks).without(up);
            let res = analyze_dealer(AnalyzeDealerRequest {
                version: 1,
                rules: rules.clone(),
                shoe: *shoe.counts(),
                up,
                hole: None,
                drawn: Vec::new(),
            })
            .unwrap();
            let d = res.dealer;
            let ctx = format!(
                "{} mazos, H17={}, visible {up}",
                t.decks, t.dealer_hits_soft17
            );
            for (name, got, want) in [
                ("p17", d.p17, row.p17),
                ("p18", d.p18, row.p18),
                ("p19", d.p19, row.p19),
                ("p20", d.p20, row.p20),
                ("p21", d.p21, row.p21),
                ("pBust", d.p_bust, row.p_bust),
            ] {
                assert!(
                    (got - want).abs() < TOL,
                    "{ctx}: {name} = {got:.7}, tabla = {want}"
                );
            }
            assert_eq!(d.p_blackjack, 0.0, "{ctx}: con peek no hay blackjack");
            checked += 1;
        }
    }
    assert_eq!(checked, 160);
}

/// Valores de la tabla de ejemplo del documento de arquitectura.
#[test]
fn doc_examples() {
    let cases = [
        (1, false, 5, 0.420823),
        (6, false, 0, 0.167033),
        (6, false, 1, 0.353504),
        (6, false, 5, 0.422842),
        (6, false, 9, 0.230239),
        (6, true, 0, 0.201281),
        (6, true, 5, 0.439259),
    ];
    for (decks, h17, up, bust) in cases {
        let up = Rank::new(up).unwrap();
        let rules = Rules {
            decks,
            dealer_hits_soft17: h17,
            ..Rules::default()
        };
        let res = analyze_dealer(AnalyzeDealerRequest {
            version: 1,
            rules,
            shoe: *Shoe::full(decks).without(up).counts(),
            up,
            hole: None,
            drawn: Vec::new(),
        })
        .unwrap();
        assert!(
            (res.dealer.p_bust - bust).abs() < TOL,
            "{decks}D H17={h17} {up}: {}",
            res.dealer.p_bust
        );
    }
}
