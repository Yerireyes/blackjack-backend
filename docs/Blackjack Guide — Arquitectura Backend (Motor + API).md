# Blackjack Guide — Arquitectura Backend (Motor + API)

Oct 14, 2026 · @Erik

## Contexto, alcance y regla de oro

El "backend" de este proyecto es, ante todo, un **motor de probabilidades exacto escrito en Rust**. Calcula el EV y las probabilidades de cada acción con la composición real del zapato (análisis *composition-dependent*). Se compila a WebAssembly para correr en el navegador; una API HTTP es opcional y reutiliza el mismo código.

| Pieza | Obligatoria | Qué hace |
| --- | --- | --- |
| `engine` (crate Rust) | Sí | Todo el cálculo: zapato, reglas, dealer, EV de acciones |
| `engine-wasm` | Sí | Expone el motor a JavaScript (paquete `@bj/engine-wasm`) |
| `sim` (crate Rust) | Sí | Simulador Monte Carlo para verificar el motor, nunca para la UI |
| `api` (Axum + PostgreSQL) | No | Cuentas, historial de sesiones, estadísticas; cálculo remoto opcional |

### Regla de oro del motor (no negociable)

1. Toda probabilidad sale del vector de conteos del zapato que recibe. **No existe ninguna tabla de estrategia ni probabilidad precalculada** en el código de producción.
2. Las cartas se sacan **sin reposición**: cada carta que se roba en la recursión se descuenta del zapato para la siguiente.
3. La carta oculta del dealer está dentro del zapato recibido, porque el jugador no la conoce. Cuando hay peek, el motor condiciona todo a que esa carta no forme blackjack.
4. La acción recomendada es la de **mayor EV**.
5. Todo resultado lleva `exact: true` o `false`. Solo puede ser `true` si el cálculo es exacto para esa composición y esas reglas; dividir es `false` mientras se use la aproximación descrita más abajo.
6. Las probabilidades de cada resultado deben sumar 1 con un error menor a 1e-9; si no, es un bug y el test debe fallar.

## Stack tecnológico

Rust estable para el motor, wasm-pack para llevarlo al navegador y Axum solo si se necesita servidor. Usar la última versión estable de cada crate al iniciar.

| Capa | Elección | Por qué |
| --- | --- | --- |
| Lenguaje del motor | Rust (edición 2021 o posterior) | Recursión con memoización en milisegundos; el mismo código sirve para WASM y servidor |
| Memoización | `rustc-hash` (`FxHashMap`) | Hash rápido para claves pequeñas (conteos empaquetados) |
| Serialización | `serde` + `serde_json` | Requests y responses idénticos en WASM y HTTP |
| Tipos para TypeScript | `tsify` (o `ts-rs`) | Genera `contract.ts` del front desde los structs de Rust |
| Binding WASM | `wasm-bindgen` + `wasm-pack` (target `web`) | Paquete npm consumido por el Web Worker |
| Tests | `cargo test` + `proptest` | Propiedades: probabilidades suman 1, ningún conteo negativo |
| Benchmarks | `criterion` | Medir tiempos por caso (1 y 8 mazos) |
| Aleatoriedad (solo `sim`) | `rand` + `rand_chacha` | Simulación reproducible con semilla |
| API (opcional) | Axum + Tokio | Servidor HTTP que llama al mismo crate `engine` |
| Base de datos (opcional) | PostgreSQL + `sqlx` | Consultas verificadas en compilación |
| Autenticación (opcional) | JWT en cookie httpOnly + `argon2` para contraseñas | Estándar y simple |

El motor no debe depender de nada del entorno (ni red, ni archivos, ni reloj): recibe un estado y devuelve un resultado. Eso lo hace determinista y fácil de probar.

## Estructura del workspace

Un workspace de Cargo con tres crates de Rust (más la API opcional) dentro del mismo monorepo que el frontend. Todo depende del crate `engine`, y `engine` no depende de nada.

&#91;embedded content: workspace · líneas punteadas = opcional\]

Las flechas indican "usa": el navegador llega al motor por WASM por defecto, y la API solo se agrega si hacen falta cuentas o historial.

```text
blackjack-guide/
├── Cargo.toml               # [workspace] members = engine, engine-wasm, sim, api
├── engine/
│   ├── src/
│   │   ├── lib.rs           # API pública: analyze_hand, analyze_dealer, analyze_round
│   │   ├── rank.rs          # Rank (índice 0..9), valores
│   │   ├── shoe.rs          # Shoe: conteos, draw/undo, clave empaquetada
│   │   ├── rules.rs         # Rules + validación
│   │   ├── hand.rs          # total, soft, pares, blackjack natural
│   │   ├── dealer.rs        # distribución final del dealer (+ peek)
│   │   ├── player/
│   │   │   ├── stand.rs
│   │   │   ├── hit.rs       # recursión óptima pedir/plantarse
│   │   │   ├── double.rs
│   │   │   ├── split.rs
│   │   │   └── surrender.rs
│   │   ├── hole.rs          # distribución de la carta oculta condicionada
│   │   ├── outcome.rs       # Outcome { ev, win, push, lose }
│   │   └── dto.rs           # structs serde + tsify del contrato
│   ├── tests/               # referencias publicadas, propiedades
│   └── benches/             # criterion
├── engine-wasm/
│   └── src/lib.rs           # #[wasm_bindgen] wrappers sobre engine
├── sim/
│   └── src/                 # simulador Monte Carlo + CLI de verificación
├── api/                     # opcional
│   ├── src/
│   │   ├── main.rs
│   │   ├── routes/          # analyze, auth, sessions
│   │   └── db/
│   └── migrations/          # sqlx
└── web/                     # ver doc de frontend
```

## Modelo de datos del motor

El zapato es un arreglo de 10 conteos. J, Q y K no existen para el motor: todas las cartas de valor 10 van al índice 9. Los structs del contrato llevan `serde` y `tsify` para que el front reciba exactamente los mismos tipos.

```rust
/// 0 = As, 1 = 2, ..., 8 = 9, 9 = cualquier carta de valor 10
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Rank(u8);
impl Rank { pub fn value(self) -> u8 { if self.0 == 0 { 1 } else { self.0 + 1 } } }

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Shoe { counts: [u16; 10], total: u16 }
impl Shoe {
    pub fn prob(&self, r: Rank) -> f64;      // counts[r] / total
    pub fn remove(&mut self, r: Rank);       // panic en debug si counts[r] == 0
    pub fn add(&mut self, r: Rank);
    pub fn key(&self) -> u128;               // 10 conteos empaquetados (8 bits c/u, ya que hasta 8 mazos el rango 10 tiene 128)
}

#[derive(Clone, Copy, Debug)]
pub struct HandState { total: u8, soft: bool, n_cards: u8, is_pair: Option<Rank>, from_split: bool }

#[derive(Clone, Copy, Debug, Default)]
pub struct Outcome { pub ev: f64, pub win: f64, pub push: f64, pub lose: f64 }

#[derive(Clone, Debug, Serialize, Deserialize, Tsify)]
pub struct Rules {
    pub decks: u8,
    pub dealer_hits_soft17: bool,
    pub double_after_split: bool,
    pub double_on: DoubleOn,          // Any | NineToEleven | TenToEleven
    pub max_split_hands: u8,
    pub resplit_aces: bool,
    pub hit_split_aces: bool,
    pub surrender: Surrender,         // None | Late
    pub dealer_peeks: bool,
    pub blackjack_pays: f64,          // 1.5 o 1.2
    pub penetration: f64,             // la usa el front para rebarajar; el motor la ignora
}

pub struct DealerDist { pub p: [f64; 7] } // 17, 18, 19, 20, 21, Blackjack, Bust
```

**Validación de entrada**: `analyze_*` devuelve error (no panic) si el zapato tiene un total imposible para el número de mazos, si alguna carta de la mano no puede estar en ese zapato, o si las reglas son incoherentes. El front muestra ese error tal cual.

## Algoritmos

Todo es recursión exacta sobre el zapato, sacando cartas sin reposición y memoizando por la clave del zapato. La única aproximación permitida es la de dividir, y debe salir marcada con `exact: false`.

Notación: el zapato S tiene n\_r cartas del rango r y N cartas en total. La probabilidad simple de sacar r es n\_r / N.

### 1. Distribución final del dealer

Desde la mano del dealer, si debe plantarse (17 duro, o 17 suave con S17) devuelve su total; si pasa de 21, "bust"; si no, saca cada rango posible y avanza.

```latex
F(d, S) = \sum_{r} \frac{n_r}{N} \, F(d + r,\; S - r)
```

- Memo por (total, soft, número de cartas, `S.key()`).
- Dos cartas que suman 21 cuentan como **Blackjack**, no como 21.
- H17: con 17 suave el dealer pide.

### 2. La carta oculta y el peek (el detalle que hace los números reales)

La carta oculta ya salió del zapato pero nadie la conoce, así que el zapato que recibe el motor la incluye. Si hay peek y la carta visible es As (o 10) y el dealer no tiene blackjack, la oculta **no puede** ser 10 (o As). Sea f ese rango prohibido. La distribución de la carta oculta es:

```latex
q_h(S) = \frac{n_h}{N - n_f} \quad (h \neq f), \qquad q_f(S) = 0
```

Sin peek, o con carta visible del 2 al 9, no hay rango prohibido: q\_h = n\_h / N.

Esa carta oculta cambia lo que el jugador puede sacar. En **cada nodo** de la recursión del jugador se usan estas dos fórmulas:

```latex
P(\text{siguiente} = r \mid S) = \sum_h q_h(S) \, \frac{n_r - [h = r]}{N - 1}
```

```latex
D(\text{up}, S) = \sum_h q_h(S) \, F(\text{up} + h,\; S - h)
```

Por qué es exacto: después de que el jugador saca una carta r, la probabilidad posterior de la carta oculta es justamente q calculada sobre el zapato reducido S − r (se puede verificar con Bayes). Por eso basta con recalcular q en cada nodo. Sin rango prohibido, la primera fórmula se reduce a n\_r / N, así que el mismo código sirve para todos los casos.

### 3. Plantarse

Con total t ≤ 21 frente a la distribución D:

- `win` = P(bust) + suma de P(d) con d < t
- `push` = P(t) si t ≥ 17
- `lose` = suma de P(d) con d > t, más P(Blackjack)
- `ev` = win − lose

Blackjack natural del jugador: paga `blackjack_pays` salvo que el dealer también tenga blackjack (empate). Un 21 de dos cartas tras dividir **no** es blackjack.

### 4. Pedir (juego óptimo después de pedir)

```latex
V(t, S) = \max\{\,\text{Plantarse}(t, S),\; \sum_r P(r \mid S)\, V(t + r,\; S - r)\,\}
```

- Pasarse de 21 vale −1 (`lose` = 1).
- Después de pedir solo existen pedir o plantarse.
- El EV de "Pedir" que se muestra es la rama de pedir del nodo raíz, jugando óptimamente después; `win`, `push` y `lose` se acumulan siguiendo la rama elegida en cada nodo. Empate exacto de EV → se elige plantarse.
- Memo por (total, soft, `from_split`, `S.key()`).
- `pBustOnHit` = suma de P(r) para las cartas r que pasan al jugador de 21.

### 5. Doblar

Una sola carta y luego plantarse, con apuesta doble:

```latex
\text{Doblar}(t, S) = 2 \sum_r P(r \mid S)\, \text{Plantarse}(t + r,\; S - r)
```

Solo se ofrece si lo permiten las reglas (`double_on`, DAS en manos divididas).

### 6. Rendirse

`ev` = −0,5 exacto. En la v1 solo se ofrece con `dealer_peeks = true` (rendición tardía después del peek).

### 7. Dividir (aproximado en v1)

El cálculo exacto con re-divisiones crece de forma explosiva; incluso las calculadoras profesionales usan aproximaciones. Versión 1:

- El zapato ya tiene fuera las dos cartas del par.
- Se calcula E1 = EV de una mano que empieza con una carta c, jugada de forma óptima (con doblar si hay DAS; ases divididos reciben una sola carta salvo `hit_split_aces`).
- `ev` de dividir = 2 × E1; `win`/`push`/`lose` reportados por mano; `exact: false`.
- Lo que ignora: la dependencia entre las cartas de las dos manos y las re-divisiones.

Mejoras planificadas (fases posteriores): re-división con la probabilidad de volver a sacar el mismo rango, y cálculo exacto de dos manos sin re-división (que permitiría `exact: true` cuando `max_split_hands = 2`). Toda mejora se valida contra el simulador.

### 8. Seguro (opcional)

Con As visible y antes del peek: p = n\_10 / N. EV del seguro por unidad apostada = 3p − 1. Si se implementa, agregar el campo `insurance_ev` al contrato y al front.

### 9. Ronda completa (modo dealer)

`analyze_round` ejecuta `analyze_hand` para cada mano visible con el mismo zapato y devuelve `house_edge_round` = − promedio del mejor EV de cada mano. Ignora el efecto de las cartas que sacarán los otros jugadores en el futuro; es un efecto muy pequeño pero debe documentarse en la UI como aproximación.

### 10. Rendimiento

- `f64` en todo el cálculo; nada de redondeos internos.
- Memo `FxHashMap` por llamada. El worker puede conservar una caché acotada (LRU) entre llamadas, porque el zapato cambia poco de una mano a otra. **Hecho** en `engine/src/cache.rs`: encendida en WASM, apagada en la API (ver README, «Rendimiento»).
- Objetivos a medir con `criterion` en una laptop media: `analyze_hand` sin dividir por debajo de 100 ms con 8 mazos; con dividir, por debajo de 500 ms. Si no se cumplen, optimizar antes de agregar funciones.

## API pública del motor y binding WASM

Tres funciones puras forman toda la superficie del motor. El binding WASM y la API HTTP solo las envuelven, sin lógica propia.

```rust
// engine/src/lib.rs
pub fn analyze_hand(req: AnalyzeHandRequest) -> Result<AnalyzeHandResponse, EngineError>;
pub fn analyze_dealer(req: AnalyzeDealerRequest) -> Result<AnalyzeDealerResponse, EngineError>;
pub fn analyze_round(req: AnalyzeRoundRequest) -> Result<AnalyzeRoundResponse, EngineError>;
```

Los structs de request y response (en `dto.rs`) son el **contrato** que usa el front. Sus campos, en `camelCase` para JSON:

| Struct | Campos |
| --- | --- |
| `AnalyzeHandRequest` | `version`, `rules`, `shoe: [u16; 10]` (incluye la oculta), `hand: Vec<Rank>`, `dealerUp`, `splitHandsSoFar`, `isSplitAces`, `allowed { double, split, surrender }` |
| `AnalyzeHandResponse` | `version`, `actions: Vec<ActionResult>`, `best`, `pBustOnHit`, `dealer: DealerDistribution`, `computeMs` |
| `ActionResult` | `action`, `ev`, `pWin`, `pPush`, `pLose`, `exact` |
| `DealerDistribution` | `p17`, `p18`, `p19`, `p20`, `p21`, `pBlackjack`, `pBust` |
| `AnalyzeDealerRequest` | `version`, `rules`, `shoe`, `up`, `hole?` |
| `AnalyzeRoundRequest` | `version`, `rules`, `shoe`, `up`, `hands: Vec<Vec<Rank>>` |
| `AnalyzeRoundResponse` | `version`, `hands: Vec<AnalyzeHandResponse>`, `houseEdgeRound` |

`version` se devuelve tal como llegó; el motor no la interpreta. `computeMs` se mide en el binding (no dentro del motor, que no usa reloj).

### Binding WASM

```rust
// engine-wasm/src/lib.rs
#[wasm_bindgen(js_name = analyzeHand)]
pub fn analyze_hand(req: JsValue) -> Result<JsValue, JsError> {
    let req: AnalyzeHandRequest = serde_wasm_bindgen::from_value(req)?;
    let t0 = js_sys::Date::now();
    let mut res = engine::analyze_hand(req).map_err(|e| JsError::new(&e.to_string()))?;
    res.compute_ms = js_sys::Date::now() - t0;
    Ok(serde_wasm_bindgen::to_value(&res)?)
}
// igual para analyzeDealer y analyzeRound
```

- Compilar con `wasm-pack build engine-wasm --target web --release --out-dir ../web/src/engine/pkg` (o publicarlo como paquete local del workspace `@bj/engine-wasm`).
- Activar `opt-level = 3` y `lto = true` en el perfil release; no usar `wee_alloc`.
- Generar los tipos TypeScript con `tsify` y copiarlos a `web/src/engine/contract.ts` en un script `pnpm gen:contract`. Un test de CI falla si el archivo generado difiere del que está en el repo.

## API HTTP opcional y base de datos

La API solo hace falta para cuentas, historial y estadísticas de sesión. Se construye **después** de que el motor y el front funcionen sin servidor.

### Endpoints (prefijo `/api/v1`)

| Método | Ruta | Qué hace | Auth |
| --- | --- | --- | --- |
| POST | `/analyze/hand` | Llama a `engine::analyze_hand` (mismo contrato que WASM) | No |
| POST | `/analyze/dealer` | Llama a `engine::analyze_dealer` | No |
| POST | `/analyze/round` | Llama a `engine::analyze_round` | No |
| POST | `/auth/register` | Crea usuario (email + contraseña con argon2) | No |
| POST | `/auth/login` | Devuelve JWT en cookie httpOnly | No |
| POST | `/auth/logout` | Borra la cookie | Sí |
| POST | `/sessions` | Inicia una sesión de juego con sus reglas | Sí |
| POST | `/sessions/{id}/hands` | Registra una mano: cartas, acción elegida, acción óptima, EV de ambas | Sí |
| GET | `/sessions/{id}/stats` | EV perdido acumulado, % de decisiones óptimas, resultado | Sí |
| GET | `/health` | Estado del servicio | No |

- Los endpoints `/analyze/*` corren el cálculo en `tokio::task::spawn_blocking` para no bloquear el servidor, con límite de tiempo de 5 s y límite de tasa por IP.
- Errores en formato `{ "code": "...", "message": "..." }` con códigos HTTP estándar; `EngineError` se traduce a 422.
- CORS limitado al dominio del front.

### Esquema de base de datos (PostgreSQL)

| Tabla | Columnas principales |
| --- | --- |
| `users` | `id uuid`, `email` (único), `password_hash`, `created_at` |
| `game_sessions` | `id uuid`, `user_id`, `rules jsonb`, `mode` (player/dealer), `started_at`, `ended_at` |
| `hands` | `id uuid`, `session_id`, `player_cards smallint[]`, `dealer_up`, `dealer_final smallint[]`, `chosen_action`, `best_action`, `ev_chosen double precision`, `ev_best double precision`, `exact boolean`, `result` (win/push/lose), `created_at` |

`ev_best − ev_chosen` da el EV perdido por cada decisión. Esos valores se guardan tal como los devolvió el motor, sin redondear. Migraciones con `sqlx migrate`.

## Validación y testing

Las probabilidades se dan por "reales" solo cuando pasan cuatro filtros: coinciden con tablas publicadas, cumplen propiedades matemáticas, coinciden con millones de manos simuladas y reaccionan correctamente cuando cambia la composición del zapato.

### 1. Tablas publicadas de referencia

Wizard of Odds publica la distribución final del dealer según su carta visible, para 1 a 8 mazos, S17 y H17, **asumiendo que el dealer ya revisó y no tiene blackjack** ([fuente](https://wizardofodds.com/games/blackjack/dealer-odds-blackjack-us-rules/)). Es la referencia ideal para `analyze_dealer` con peek.

Valores de prueba (probabilidad de pasarse), tomados de esa página:

| Mazos | Regla | Carta visible | P(pasarse) |
| --- | --- | --- | --- |
| 1 | S17 | 6 | 0,420823 |
| 6 | S17 | As | 0,167033 |
| 6 | S17 | 2 | 0,353504 |
| 6 | S17 | 6 | 0,422842 |
| 6 | S17 | 10 | 0,230239 |
| 6 | H17 | As | 0,201281 |
| 6 | H17 | 6 | 0,439259 |

Cómo usarla:

- Zapato = mazos completos menos la carta visible del dealer (se asume que la tabla solo retira esa carta; si un test falla por una diferencia sistemática, revisar primero este supuesto).
- Tolerancia 1e-6, porque la tabla tiene 6 decimales.
- Cargar **todas** las filas de las 16 tablas como fixture JSON (`engine/tests/fixtures/woo_dealer.json`) y probarlas todas, no solo estos ejemplos.
- Para EV del jugador, usar como referencia el apéndice de valores esperados dependientes de la composición para 1 a 8 mazos del mismo sitio ([fuente](https://wizardofodds.com/games/blackjack/expected-values/)).

### 2. Propiedades (proptest)

Con zapatos y manos aleatorias válidas:

- `win + push + lose = 1` en cada acción, y la distribución del dealer suma 1 (error < 1e-9).
- `ev` de plantarse está entre −1 y +1; doblar entre −2 y +2.
- `best` es siempre la acción con mayor `ev` dentro de `actions`.
- Con peek y As visible, `pBlackjack = 0`.
- Ningún conteo del zapato queda negativo durante la recursión.

### 3. Verificación cruzada con Monte Carlo (crate `sim`)

- Para un estado dado (zapato, mano, carta visible), el simulador reparte la carta oculta y las siguientes al azar desde ese mismo zapato, respetando el peek, y juega con la estrategia que eligió el motor en cada nodo.
- Con 10 millones de manos, el EV simulado debe quedar dentro de 4 errores estándar del EV exacto. Si no, hay un bug en el motor o en el simulador.
- Es la única forma de validar dividir: el EV aproximado debe quedar muy cerca del simulado; documentar la diferencia observada.
- Se ejecuta con un comando aparte (`cargo run -p sim --release -- verify`), no en cada `cargo test`.

### 4. Tests de composición

- Quitar todos los 10 del zapato: la probabilidad de que el dealer se pase con 6 debe bajar.
- Quitar todas las cartas del 6 al 10: con un 16 duro, `pBustOnHit` debe ser exactamente 0.
- 1 mazo, mano 10-2 contra 4 frente a 8 mazos: los EV deben diferir (efecto de eliminación de cartas).
- Mismo estado pedido dos veces: resultado idéntico bit a bit (determinismo).

### 5. Rendimiento

Benchmarks con `criterion` para 1, 2, 6 y 8 mazos; CI falla si un caso empeora más de un 20 % respecto de la medición anterior guardada.

## Plan de implementación para Claude Code

Construir de adentro hacia afuera: primero el cálculo del dealer validado contra las tablas publicadas, luego el jugador, luego el binding y al final la API. Ninguna fase avanza con tests en rojo.

1. **Workspace y tipos base**: `Cargo.toml` del workspace, crate `engine` con `Rank`, `Shoe`, `Rules`, `HandState`, `Outcome` y validación de entrada. Tests de `Shoe` (remove/add, clave empaquetada).
2. **Dealer**: `dealer.rs` con S17/H17 y blackjack separado; `hole.rs` con el condicionamiento por peek. Fixture con las 16 tablas de Wizard of Odds y test que las recorre todas.
3. **Plantarse, pedir y doblar**: recursión óptima con las dos fórmulas de la carta oculta en cada nodo, `pBustOnHit`, propiedades con `proptest`.
4. **Simulador `sim`**: Monte Carlo reproducible y comando `verify`. Confirmar plantarse, pedir y doblar dentro de 4 errores estándar antes de seguir.
5. **Rendirse y dividir (v1 aproximado)**: `exact: false` en dividir; comparar contra el simulador y anotar la diferencia en el README del motor.
6. **API pública + DTOs**: `analyze_hand`, `analyze_dealer`, `analyze_round`, `serde` + `tsify`, script `gen:contract`.
7. **Binding WASM**: `engine-wasm` con wasm-pack, perfil release optimizado, prueba de humo desde Node. Desde aquí el front puede pasar de su mock al motor real (fase 4 del doc de frontend).
8. **Rendimiento**: benchmarks `criterion`, caché LRU opcional y optimización hasta cumplir los objetivos.
9. **Mejoras de dividir**: re-split y, si es viable, cálculo exacto de dos manos; siempre validado con `sim`.
10. **API HTTP (opcional)**: Axum, endpoints `/analyze/*`, luego auth, sesiones, manos y estadísticas con PostgreSQL y `sqlx`.

Al pasarle este documento a Claude Code junto con el de frontend, pídele empezar por las fases 1 a 4 de este documento y, en paralelo, las fases 1 a 3 del frontend usando el mock del contrato.
