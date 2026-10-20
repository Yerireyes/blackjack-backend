//! CLI de verificación: `cargo run -p sim --release -- verify`
//!
//! Para cada caso compara el EV exacto del motor con el EV simulado. Plantarse,
//! pedir y doblar deben quedar dentro de 4 errores estándar; dividir es una
//! aproximación y solo se reporta la diferencia observada.

use std::process::ExitCode;
use std::time::Instant;

use clap::{Parser, Subcommand};
use engine::{analyze_hand, Action, Allowed, AnalyzeHandRequest, Rank, Rules, Shoe, Surrender};
use sim::{simulate, Case};

#[derive(Parser)]
#[command(about = "Simulador Monte Carlo para verificar el motor de blackjack")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Verifica el motor contra la simulación en un conjunto de casos.
    Verify {
        /// Manos simuladas por caso y acción.
        #[arg(long, default_value_t = 10_000_000)]
        hands: u64,
        #[arg(long, default_value_t = 20261014)]
        seed: u64,
        /// Hilos (por defecto, todos los núcleos).
        #[arg(long)]
        threads: Option<usize>,
        /// Máximo de errores estándar permitido.
        #[arg(long, default_value_t = 4.0)]
        max_z: f64,
    },
}

fn r(i: u8) -> Rank {
    Rank::new(i).unwrap()
}

fn case(name: &str, rules: Rules, hand: &[Rank], up: Rank, burn: &[(Rank, u16)]) -> Case {
    let mut shoe = Shoe::full(rules.decks);
    for &c in hand.iter().chain([&up]) {
        shoe.remove(c);
    }
    for &(rank, k) in burn {
        for _ in 0..k {
            shoe.remove(rank);
        }
    }
    Case {
        name: name.to_string(),
        rules,
        shoe,
        hand: hand.to_vec(),
        up,
    }
}

fn cases() -> Vec<Case> {
    let s17 = |decks| Rules {
        decks,
        surrender: Surrender::Late,
        ..Rules::default()
    };
    let h17 = |decks| Rules {
        decks,
        dealer_hits_soft17: true,
        ..Rules::default()
    };
    let no_peek = |decks| Rules {
        decks,
        dealer_peeks: false,
        ..Rules::default()
    };
    let two = |rules: Rules| Rules {
        max_split_hands: 2,
        ..rules
    };
    let rsa = |rules: Rules| Rules {
        resplit_aces: true,
        ..rules
    };
    vec![
        case(
            "6D S17  10,6 vs 10",
            s17(6),
            &[Rank::TEN, r(5)],
            Rank::TEN,
            &[],
        ),
        case("1D H17  6,5 vs 6", h17(1), &[r(5), r(4)], r(5), &[]),
        case("8D S17  A,7 vs 9", s17(8), &[Rank::ACE, r(6)], r(8), &[]),
        case(
            "2D noPeek 10,2 vs A",
            no_peek(2),
            &[Rank::TEN, r(1)],
            Rank::ACE,
            &[],
        ),
        case("6D H17  9,4 vs A", h17(6), &[r(8), r(3)], Rank::ACE, &[]),
        case(
            "1D S17  2,2,2 vs 10",
            s17(1),
            &[r(1), r(1), r(1)],
            Rank::TEN,
            &[],
        ),
        case(
            "6D S17  10,5 vs 4 sin 20 dieces",
            s17(6),
            &[Rank::TEN, r(4)],
            r(3),
            &[(Rank::TEN, 20)],
        ),
        case("1D S17  8,8 vs 10", s17(1), &[r(7), r(7)], Rank::TEN, &[]),
        case(
            "6D S17  A,A vs 6",
            s17(6),
            &[Rank::ACE, Rank::ACE],
            r(5),
            &[],
        ),
        case("2D H17  9,9 vs 9", h17(2), &[r(8), r(8)], r(8), &[]),
        // Dos manos como máximo: dividir debe ser exacto.
        case(
            "1D S17  8,8 vs 10 (2 manos)",
            two(s17(1)),
            &[r(7), r(7)],
            Rank::TEN,
            &[],
        ),
        case(
            "6D S17  7,7 vs 6 (2 manos)",
            two(s17(6)),
            &[r(6), r(6)],
            r(5),
            &[],
        ),
        case(
            "1D H17  6,6 vs 5 (2 manos)",
            two(h17(1)),
            &[r(5), r(5)],
            r(4),
            &[],
        ),
        case(
            "2D noPeek 9,9 vs 8 (2 manos)",
            two(no_peek(2)),
            &[r(8), r(8)],
            r(7),
            &[],
        ),
        // Re-división hasta 4 manos (aproximado).
        case("6D S17  2,2 vs 4", s17(6), &[r(1), r(1)], r(3), &[]),
        case(
            "1D S17  A,A vs 10 RSA",
            rsa(s17(1)),
            &[Rank::ACE, Rank::ACE],
            Rank::TEN,
            &[],
        ),
    ]
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let Command::Verify {
        hands,
        seed,
        threads,
        max_z,
    } = cli.command;
    let threads =
        threads.unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get()));
    println!("verify: {hands} manos por caso y acción, semilla {seed}, {threads} hilos\n");
    println!(
        "{:<34} {:<9} {:>10} {:>10} {:>9} {:>7}",
        "caso", "acción", "exacto", "simulado", "err.est", "z"
    );

    let mut failures = 0;
    let t0 = Instant::now();
    for c in cases() {
        let res = analyze_hand(AnalyzeHandRequest {
            version: 1,
            rules: c.rules.clone(),
            shoe: *c.shoe.counts(),
            hand: c.hand.clone(),
            dealer_up: c.up,
            split_hands_so_far: 0,
            is_split_aces: false,
            allowed: Allowed::default(),
        })
        .expect("caso válido");
        for a in res.actions.iter().filter(|a| a.action != Action::Surrender) {
            let s = simulate(&c, a.action, hands, seed, threads);
            let z = (s.mean() - a.ev) / s.std_err();
            let verdict = if !a.exact {
                format!("aprox. (diferencia {:+.5})", a.ev - s.mean())
            } else if z.abs() <= max_z {
                "OK".to_string()
            } else {
                failures += 1;
                "FALLA".to_string()
            };
            println!(
                "{:<34} {:<9} {:>+10.5} {:>+10.5} {:>9.5} {:>+7.2}  {}",
                c.name,
                format!("{:?}", a.action),
                a.ev,
                s.mean(),
                s.std_err(),
                z,
                verdict
            );
        }
    }
    println!("\n{:.1} s", t0.elapsed().as_secs_f64());
    if failures == 0 {
        println!("Todas las acciones exactas dentro de {max_z} errores estándar.");
        ExitCode::SUCCESS
    } else {
        println!("{failures} acción(es) fuera de {max_z} errores estándar: hay un bug en el motor o en el simulador.");
        ExitCode::FAILURE
    }
}
