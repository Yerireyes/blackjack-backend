# Blackjack Guide — Backend (motor + WASM + API)

Motor exacto de probabilidades de blackjack en Rust, *composition-dependent*: todo
sale del vector de conteos del zapato, sin tablas precalculadas. Se compila a
WebAssembly (`@bj/engine-wasm`) para el navegador; la API HTTP (Axum + PostgreSQL)
es opcional y reutiliza el mismo crate. Diseño completo en
[docs/Blackjack Guide — Arquitectura Backend (Motor + API).md](docs/).

| Crate / carpeta | Qué hace |
| --- | --- |
| `engine/` | Todo el cálculo: zapato, reglas, dealer, EV de acciones, contrato (`dto.rs`) |
| `engine-wasm/` | Wrappers `#[wasm_bindgen]`: `analyzeHand`, `analyzeDealer`, `analyzeRound` |
| `sim/` | Monte Carlo para verificar el motor (`verify`) |
| `api/` | Axum + PostgreSQL: `/analyze/*`, cuentas, sesiones y estadísticas |
| `contract/contract.ts` | Tipos TS generados desde los structs de Rust (no editar a mano) |
| `.sqlx/` | Metadatos de las consultas SQL para compilar sin base de datos |
| `scripts/` | Build WASM, contrato, prueba de humo, PostgreSQL local |

## Requisitos (Windows, ya instalados en esta máquina)

- Rust estable, toolchain `x86_64-pc-windows-gnu` + target `wasm32-unknown-unknown`.
- MinGW-w64 (WinLibs) en el `PATH` (`dlltool`/`gcc` para el toolchain GNU).
- `wasm-pack` (necesita el Visual C++ Redistributable x64).
- `sqlx-cli` (`cargo install sqlx-cli --no-default-features --features postgres,rustls`).
- PostgreSQL 18 portable en `%LOCALAPPDATA%\Programs\PostgreSQL18` (sin servicio de
  Windows: hay que arrancarlo con `npm run db:start` después de reiniciar el equipo).
- Node 18+ para los scripts.

Configuración en `.env` (no se versiona; plantilla en [.env.example](.env.example)).

## Comandos

```sh
npm run db:start         # arranca PostgreSQL local (db:stop / db:status)
npm run api              # API en http://127.0.0.1:8080/api/v1 (aplica migraciones al arrancar)
npm test                 # cargo test --workspace (los tests de la API necesitan la base arrancada)
npm run test:engine      # solo el motor
npm run verify           # Monte Carlo, 10 M de manos por caso y acción
npm run bench            # criterion, 1/2/6/8 mazos
npm run build:wasm       # wasm-pack → engine-wasm/pkg (@bj/engine-wasm, target web)
npm run gen:contract     # contract/contract.ts (+ ../Frontend/src/engine/contract.ts si existe el front)
npm run check:contract   # falla si contract.ts no coincide con Rust (para CI)
npm run smoke:wasm       # carga el .wasm en Node y llama las tres funciones
npm run db:migrate       # aplica migraciones con sqlx-cli
npm run sqlx:prepare     # regenera .sqlx tras cambiar consultas SQL
```

Uso del motor desde el front (Web Worker), con la dependencia
`"@bj/engine-wasm": "file:../Backend/engine-wasm/pkg"`:

```ts
import init, { analyzeHand } from "@bj/engine-wasm";
await init();
const res = analyzeHand({ version: 1, rules, shoe, hand: [9, 5], dealerUp: 9 });
```

## Contrato: puntos que el front debe respetar

- **Rangos**: `0` = As, `1` = 2 … `8` = 9, `9` = cualquier carta de valor 10.
- **`shoe`**: cartas **sin ver**. Incluye la carta oculta del dealer y excluye todas las
  visibles (mano del jugador, carta visible del dealer, otras manos de la mesa).
  En `analyzeDealer` con `hole` informada, el zapato **no** la incluye.
- **`analyzeDealer.drawn`** (opcional, requiere `hole`): cartas que el dealer ya sacó
  después de la oculta; tampoco van en el zapato. La respuesta trae `pBustNext`
  (probabilidad de pasarse con la próxima carta) solo cuando las reglas obligan al
  dealer a pedir. Lo usa el juego guiado del modo dealer.
- **`splitHandsSoFar`**: cuántas veces se dividió ya en la ronda (0 = mano normal). Se
  puede dividir si `splitHandsSoFar + 2 <= maxSplitHands`.
- **Errores**: entradas imposibles lanzan una excepción (WASM) o devuelven 422 (HTTP)
  con mensaje en español.
- `computeMs` lo mide el binding; el motor no usa reloj.

## Decisiones y supuestos del motor

- **Peek**: con peek y As/10 visible, todo se condiciona a que el dealer no tenga
  blackjack. La distribución de la siguiente carta del jugador se recalcula en cada
  nodo con la forma cerrada de `sum_h q_h (n_r − [h=r]) / (N − 1)` (`engine/src/hole.rs`).
- **Sin peek**: si el dealer tiene blackjack se pierde todo lo apostado, incluidas las
  dobladas y divididas (regla europea ENHC).
- **Rendición**: solo tardía y solo con `dealerPeeks = true`.
- **Zapato mínimo**: suma de valores ≥ 60, así la recursión nunca se queda sin cartas.
- **Empates de EV**: gana la acción que va primero (plantarse, pedir, doblar, dividir,
  rendirse).
- **Seguro**: no implementado (opcional en el diseño).

### Dividir (fase 9)

Después de dividir, cada mano se juega con la **estrategia post-división**: la decisión
óptima para esa mano con el zapato de justo después de dividir menos sus propias cartas
(no usa las cartas que recibió la otra mano; el jugador real, que sí las ve, solo puede
hacerlo un poco mejor: en las pruebas la diferencia fue del orden de 1e‑7).

- **Dos manos, sin re-división → exacto** (`exact: true`): se recorren todos los estados
  finales de la mano 1, desde cada uno todos los de la mano 2 (sacando las cartas del
  zapato real), y el dealer juega con el zapato que queda. Aplica cuando
  `maxSplitHands = 2`, o con ases sin `resplitAces`, y la mano no viene de otra división.
- **Presupuesto**: el exacto se calcula si el trabajo estimado (combinaciones mano 1 ×
  mano 2 y distribuciones del dealer por zapato final) cabe en
  `SPLIT_EXACT_BUDGET` (`engine/src/player/split.rs`). Es un conteo, no un reloj, así
  que el resultado es determinista. Si no cabe (típico: pares bajos contra A, 7 o 10),
  se usa 2 × E1 y sale `exact: false`. Con este presupuesto ningún caso pasa de ~300 ms.
- **Re-división → aproximada** (`exact: false`): número esperado de manos con la
  probabilidad de volver a sacar el mismo rango (se re-divide siempre que se puede),
  sumando esa ganancia al resultado de dos manos.

Verificación contra la simulación (10 M de manos; el simulador juega la misma
estrategia, con re-división real):

| Caso | Motor | Simulado | Error est. | Resultado |
| --- | --- | --- | --- | --- |
| 1D S17 8,8 vs 10, 2 manos | −0,45174 | −0,45186 | 0,00055 | exacto, z = −0,23 |
| 6D S17 7,7 vs 6, 2 manos | +0,19273 | +0,19294 | 0,00077 | exacto, z = +0,28 |
| 1D H17 6,6 vs 5, 2 manos | +0,24383 | +0,24404 | 0,00083 | exacto, z = +0,25 |
| 2D sin peek 9,9 vs 8, 2 manos | +0,20464 | +0,20537 | 0,00052 | exacto, z = +1,40 |
| 6D S17 A,A vs 6 (sin RSA) | +0,68237 | +0,68194 | 0,00049 | exacto, z = −0,87 |
| 1D S17 8,8 vs 10, hasta 4 manos | −0,44724 | −0,44695 | 0,00058 | aprox., dif. −0,00029 |
| 2D H17 9,9 vs 9, hasta 4 manos | −0,08802 | −0,08898 | 0,00054 | aprox., dif. +0,00096 |
| 6D S17 2,2 vs 4, hasta 4 manos | +0,06942 | +0,06888 | 0,00074 | aprox., dif. +0,00054 |
| 1D S17 A,A vs 10, RSA | +0,25919 | +0,25937 | 0,00050 | aprox., dif. −0,00018 |

## Validación del motor

1. **Tablas publicadas**: las 16 tablas de Wizard of Odds (1–8 mazos, S17/H17, 160 filas)
   en `engine/tests/fixtures/woo_dealer.json`, tolerancia 1e‑6. Todas pasan.
2. **Propiedades** (`proptest`): probabilidades suman 1 (< 1e‑9), rangos de EV, `best`
   es el máximo, `pBlackjack = 0` con peek, sin conteos negativos.
3. **Monte Carlo** (`npm run verify`, 10 M manos): todas las acciones exactas
   (plantarse, pedir, doblar y dividir en dos manos) dentro de 4 errores estándar.
4. **Composición**: sin dieces baja el bust del dealer con 6; sin cartas 6–10 un 16 duro
   tiene `pBustOnHit = 0`; 1 vs 8 mazos difieren; determinismo bit a bit; reproduce
   decisiones clásicas de estrategia básica sin tablas.

## API HTTP (`/api/v1`)

| Método | Ruta | Qué hace | Auth |
| --- | --- | --- | --- |
| POST | `/analyze/hand` · `/analyze/dealer` · `/analyze/round` | Mismo contrato que WASM | No |
| POST | `/auth/register` | Crea usuario (argon2id) y deja la sesión iniciada | No |
| POST | `/auth/login` | JWT (HS256, 7 días) en cookie `bj_session` httpOnly | No |
| POST | `/auth/logout` | Borra la cookie | Sí |
| GET | `/auth/me` | Usuario de la sesión actual | Sí |
| POST · GET | `/sessions` | Crea una sesión con sus reglas · lista las del usuario | Sí |
| POST | `/sessions/{id}/hands` | Registra una mano (cartas, acción elegida y óptima, EVs) | Sí |
| GET | `/sessions/{id}/stats` | Manos, % óptimas, EV perdido, decisiones aproximadas, W/P/L | Sí |
| POST | `/sessions/{id}/end` | Cierra la sesión (ya no admite manos) | Sí |
| GET | `/health` | Estado del servicio y de la base | No |

- Errores siempre como `{ "code": "...", "message": "..." }`; `EngineError` → 422.
- `/analyze/*` corre en `spawn_blocking` con límite de 5 s y límite de tasa por IP
  (`ANALYZE_REPLENISH_MS`, `ANALYZE_BURST`); login y registro tienen su propio límite.
- CORS solo para `FRONTEND_ORIGIN`, con credenciales (el front debe usar
  `fetch(..., { credentials: "include" })`).
- Una sesión ajena responde 404 (no revela que existe).
- Cuerpo máximo 64 KiB; timeout global de 10 s.
- Tests de punta a punta en `api/tests/http.rs` con `#[sqlx::test]` (una base nueva por
  test; el rol `blackjack` tiene `CREATEDB` para eso).

## Rendimiento

Medido con `cargo bench` en la máquina de desarrollo: `analyze_hand` sin dividir
~55–70 ms con 1–8 mazos (objetivo < 100 ms); con dividir, ≤ ~300 ms en el peor caso
gracias al presupuesto del exacto (objetivo < 500 ms). En WASM, 10,6 vs 10 con 6
mazos tarda ~1–2 ms.

## Pendiente

- Seguro (`insuranceEv`) y caché LRU entre llamadas en el worker.
- CI: `cargo test`, `npm run check:contract`, `SQLX_OFFLINE=true` y comparación de
  benchmarks (> 20 %).
