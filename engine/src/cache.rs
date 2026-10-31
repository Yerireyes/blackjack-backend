//! Caché acotada de [`Solver`] entre llamadas, una por hilo. Encendida por defecto solo
//! en WASM (el Web Worker del motor en el navegador).
//!
//! La memoización del `Solver` usa como clave la composición exacta del zapato, así que
//! reutilizarlo entre llamadas da los mismos resultados bit a bit: un nodo ya calculado
//! vale lo mismo en cualquier llamada. Sirve porque el zapato cambia poco de una
//! decisión a la siguiente: después de pedir, el zapato nuevo ya fue un nodo de la
//! llamada anterior, y repetir la misma pregunta (un re-render) es gratis.
//!
//! Hay un `Solver` por reglas y carta visible del dealer, con reemplazo LRU entre
//! [`SLOTS`] de ellos. Si entre todos pasan de [`MAX_NODES`] nodos, se descartan los
//! usados hace más tiempo (y, si el actual solo ya pasa el tope, se vacía): así la
//! memoria queda acotada. En WASM importa porque la memoria nunca se devuelve al sistema.

use std::cell::RefCell;

use crate::player::Solver;
use crate::rank::Rank;
use crate::rules::Rules;

/// Combinaciones de reglas y carta visible que se conservan. Hay 10 cartas visibles
/// posibles: con 10 entra una mesa entera con las mismas reglas.
pub const SLOTS: usize = 10;

/// Nodos memoizados entre todos los `Solver` (cada uno ocupa unos 80 bytes con el mapa:
/// ~50 MB). El cálculo más grande medido (3,3 contra 6 con 8 mazos) deja ~345 mil.
pub const MAX_NODES: usize = 600_000;

struct Slot {
    rules: Rules,
    up: Rank,
    solver: Solver,
    used: u64,
}

pub struct SolverCache {
    slots: Vec<Slot>,
    capacity: usize,
    max_nodes: usize,
    clock: u64,
    enabled: bool,
}

impl SolverCache {
    pub fn new(capacity: usize, max_nodes: usize) -> SolverCache {
        SolverCache {
            slots: Vec::with_capacity(capacity),
            capacity,
            max_nodes,
            clock: 0,
            // En el navegador hay un solo hilo (el Web Worker). En la API cada hilo
            // bloqueante tendría su propia caché de hasta ~50 MB, y Tokio abre muchos bajo
            // carga: ahí queda apagada salvo que se encienda con `set_enabled`.
            enabled: cfg!(target_arch = "wasm32"),
        }
    }

    /// El `Solver` de estas reglas (validadas) y carta visible; lo crea si no está,
    /// reemplazando el usado hace más tiempo. Antes de devolverlo hace lugar si entre
    /// todos pasan el tope de nodos.
    pub fn get(&mut self, rules: &Rules, up: Rank) -> &mut Solver {
        self.clock += 1;
        let mut i = match self
            .slots
            .iter()
            .position(|s| s.up == up && s.rules == *rules)
        {
            Some(i) => i,
            None => {
                let slot = Slot {
                    rules: rules.clone(),
                    up,
                    solver: Solver::new(rules, up),
                    used: 0,
                };
                if self.slots.len() < self.capacity {
                    self.slots.push(slot);
                    self.slots.len() - 1
                } else {
                    let lru = self
                        .slots
                        .iter()
                        .enumerate()
                        .min_by_key(|(_, s)| s.used)
                        .map(|(i, _)| i)
                        .expect("la caché tiene al menos un lugar");
                    self.slots[lru] = slot;
                    lru
                }
            }
        };
        self.slots[i].used = self.clock;
        // Primero se descartan los otros, del usado hace más tiempo al más reciente.
        while self.stats().1 > self.max_nodes && self.slots.len() > 1 {
            let lru = self
                .slots
                .iter()
                .enumerate()
                .filter(|&(j, _)| j != i)
                .min_by_key(|(_, s)| s.used)
                .map(|(j, _)| j)
                .expect("hay otro además del actual");
            self.slots.swap_remove(lru);
            // `swap_remove` mueve el último al lugar del quitado.
            if i == self.slots.len() {
                i = lru;
            }
        }
        if self.slots[i].solver.memo_len() > self.max_nodes {
            self.slots[i].solver = Solver::new(rules, up);
        }
        &mut self.slots[i].solver
    }

    /// `Solver`s guardados y nodos memoizados en total.
    pub fn stats(&self) -> (usize, usize) {
        let nodes = self.slots.iter().map(|s| s.solver.memo_len()).sum();
        (self.slots.len(), nodes)
    }

    pub fn clear(&mut self) {
        self.slots.clear();
    }
}

thread_local! {
    static CACHE: RefCell<SolverCache> = RefCell::new(SolverCache::new(SLOTS, MAX_NODES));
}

/// Corre `f` con el `Solver` de la caché del hilo para estas reglas (ya validadas) y
/// carta visible. Con la caché desactivada usa uno nuevo.
pub fn with_solver<T>(rules: &Rules, up: Rank, f: impl FnOnce(&mut Solver) -> T) -> T {
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.enabled && cache.capacity > 0 {
            f(cache.get(rules, up))
        } else {
            drop(cache);
            f(&mut Solver::new(rules, up))
        }
    })
}

/// Activa o desactiva la caché de este hilo (los benchmarks la apagan para medir en
/// frío). Al desactivarla se vacía.
pub fn set_enabled(enabled: bool) {
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.enabled = enabled;
        if !enabled {
            cache.clear();
        }
    });
}

/// Vacía la caché de este hilo.
pub fn clear() {
    CACHE.with(|cache| cache.borrow_mut().clear());
}

/// `Solver`s y nodos en la caché de este hilo.
pub fn stats() -> (usize, usize) {
    CACHE.with(|cache| cache.borrow().stats())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(decks: u8) -> Rules {
        Rules {
            decks,
            ..Rules::default()
        }
    }

    #[test]
    fn keeps_one_solver_per_rules_and_up_card_and_evicts_the_oldest() {
        let mut c = SolverCache::new(2, MAX_NODES);
        let (r6, r8) = (rules(6), rules(8));
        let ten = Rank::TEN;
        c.get(&r6, ten);
        c.get(&r6, ten);
        assert_eq!(c.stats().0, 1, "misma combinación, mismo Solver");
        c.get(&r8, ten);
        c.get(&r6, ten); // r6 pasa a ser el más reciente
        c.get(&r6, Rank::ACE); // reemplaza a r8
        assert_eq!(c.stats().0, 2);
        assert!(c.slots.iter().all(|s| s.rules == r6));
    }

    #[test]
    fn a_solver_over_the_node_limit_starts_over() {
        let mut c = SolverCache::new(2, 10);
        let r = rules(6);
        let shoe = crate::Shoe::full(6);
        c.get(&r, Rank::TEN).dealer_dist(&shoe);
        assert!(c.stats().1 > 10);
        c.get(&r, Rank::TEN);
        assert_eq!(c.stats().1, 0, "se vació al pasar el tope");
    }

    #[test]
    fn over_the_limit_the_others_go_first() {
        let shoe = crate::Shoe::full(6);
        let r = rules(6);
        // El tope justo para lo que deja el dealer con un 10 visible.
        let mut probe = SolverCache::new(1, usize::MAX);
        probe.get(&r, Rank::TEN).dealer_dist(&shoe);
        let ten_nodes = probe.stats().1;
        let mut c = SolverCache::new(4, ten_nodes);
        c.get(&r, Rank::TEN).dealer_dist(&shoe);
        c.get(&r, Rank::ACE).dealer_dist(&shoe);
        assert!(c.stats().1 > ten_nodes);
        // Pedir el del 10 descarta al del As (el otro) y conserva lo del 10.
        c.get(&r, Rank::TEN);
        assert_eq!(c.stats(), (1, ten_nodes));
    }
}
