# Front — Fase 3: Casual

**Versión del 2026-10-28 (revisión 2).** Cambios respecto de la primera versión:

- `kind: "ranked"` ya no responde 501 `not_available`: Ranked existe (ver «Front — Fase 4
  Ranked.md»).
- Una ronda sin acciones durante 24 h se cierra sola plantándose (sección 3, paso 2).
- `MyRecords` trae también `ranked` (sección 6).
- `Retry-After` del 429 nunca es 0 (mínimo 1 s).

Primera versión (2026-10-28). Qué cambió respecto de lo que el front conocía:

- Nuevo modo **Casual**: el servidor baraja, reparte, valida cada acción, juega el dealer y
  liquida. Endpoints nuevos `/casual/*`, `/play/*` y `/me/records`.
- Tipos nuevos en `src/engine/contract.ts` (generados, no se copian a mano): `RoundView`,
  `CasualRunView`, `WeeklyLeaderboard`, `MyRecords` y sus partes.
- Para jugar Casual el nombre no puede ser provisional (`display_name_required`).

La fase 2 (motor de juego en el servidor) no le pide nada al front. El detalle de producto
está en las secciones 3, 4, 7 y 8.1 de
[Especificación — Cuentas, Casual y Ranked.md](Especificación%20—%20Cuentas,%20Casual%20y%20Ranked.md).
Este documento solo existe en `blackjack-backend/docs/`.

## Estado

| | Backend | Front |
|---|---|---|
| Partidas Casual (crear, retomar, retirarse) | Listo | Por hacer |
| Rondas en el servidor (`/play/*`) | Listo | Por hacer |
| Ranking semanal y récords | Listo | Por hacer |
| Revisión de decisiones al liquidar | Solo en Ranked | No en Casual |
| Ranked (fase 4) | Listo | Ver «Front — Fase 4 Ranked.md» |

Práctica no cambia: sigue en el navegador y sin cuenta.

---

## Qué añadir

### 1. Tipos

Importarlos de `@/engine/contract` (los escribe `npm run gen:contract` en el backend). Los
principales:

- `RoundView`: lo único que el cliente ve de una ronda.
- `CasualRunView`: la partida (reglas, saldo, pico, manos, estado).
- `NewRoundRequest`, `RoundActionRequest`, `CreateCasualRunRequest`: cuerpos de las peticiones.
- `WeeklyLeaderboard` y `LeaderboardRow`; `MyRecords` y `CasualRecords`.
- `CardView { rank: Face; suit: Suit }`: `rank` es la figura (`"A"`, `"2"` … `"10"`, `"J"`,
  `"Q"`, `"K"`), no el índice del motor.

Todos los importes son fichas enteras (`number`).

### 2. Endpoints (todos bajo `/api/v1`, con sesión salvo el ranking)

| Método | Ruta | Cuerpo | Éxito | Errores (`code`) |
|---|---|---|---|---|
| POST | `/casual/runs` | `{ preset }` **o** `{ rules }` | 201 `CasualRunView` | 400 `bad_request` (ninguno o los dos), 422 `engine_error` (reglas inválidas), 409 `run_active`, 409 `display_name_required` |
| GET | `/casual/runs/active` | — | 200 `CasualRunView` | 404 `no_active_run` |
| POST | `/casual/runs/{id}/retire` | — | 200 `CasualRunView` (`retired`) | 404 `not_found`, 409 `run_finished`, 409 `round_active` |
| POST | `/play/rounds` | `{ kind: "casual", contextId?, bet, idempotencyKey }` | 201 `RoundView` | 400 `bad_request` (clave o id no UUID), 400 `bet_out_of_range`, 409 `insufficient_funds`, 409 `round_active`, 409 `run_finished`, 404 `no_active_run` / `not_found` |
| POST | `/play/rounds/{id}/actions` | `{ action, idempotencyKey }` | 200 `RoundView` | 409 `action_not_allowed`, 409 `no_active_round` (ya liquidada), 404 `not_found` |
| GET | `/play/active` | — | 200 `RoundView` | 404 `no_active_round` |
| GET | `/casual/leaderboard/weekly` | — | 200 `WeeklyLeaderboard` | — (sin sesión no trae `me`) |
| GET | `/me/records` | — | 200 `MyRecords` | — |

- `preset`: `"vegas"`, `"europe"` o `"sixToFive"` (las mismas reglas que los presets del
  front). `rules`: el mismo objeto `Rules` de `/analyze/*`.
- `bet`: de 10 a 500, múltiplo de 10 y nunca más que el saldo. `wallet.maxBet` y
  `CasualRunView.maxBet` ya traen el tope para la próxima ronda.
- `action`: `"hit"`, `"stand"`, `"double"`, `"split"` o `"surrender"`.
- `contextId`: el `id` de la partida. Es opcional: sin él se usa la partida activa.
- **429 `rate_limited`:** crear o cerrar partidas, repartir y actuar comparten un límite
  **por usuario** (5 por segundo, ráfaga de 10). Trae `Retry-After`.

### 3. Flujo de una partida

1. **Antes de empezar**, si `me.needsDisplayName` es `true`, pedir el nombre (`PATCH /me`).
   Si no, `POST /casual/runs` responde `display_name_required`.
2. **Al entrar al modo Casual:** pedir `GET /casual/runs/active` y `GET /play/active` en
   paralelo.
   - Sin partida (404): ofrecer elegir reglas y `POST /casual/runs`.
   - Con ronda activa (200): **retomarla**: mostrarla tal cual y seguir desde ahí. Pasa si
     el usuario recargó o perdió la conexión a mitad de una mano. Si pasan 24 h sin
     acciones, el servidor la cierra plantándose y ya no aparece.
   - La ronda activa puede ser de Ranked (`kind: "ranked"`): entonces hay que retomarla en
     Ranked; repartir en Casual daría `round_active`.
3. **Apostar y repartir:** `POST /play/rounds`. Si la ronda se resuelve en el reparto
   (blackjack del jugador o del dealer con peek), ya viene con `phase: "settled"`.
4. **Decidir:** con `phase: "player"`, mostrar solo los botones de `allowed` y mandar cada
   decisión con `POST /play/rounds/{id}/actions`. Cada respuesta es el `RoundView`
   completo: reemplazar el estado local, no aplicarle cambios.
5. **Liquidación:** cuando la respuesta trae `phase: "settled"`, el dealer ya jugó.
   Animar `dealer.hole` y luego `dealer.hits` en orden, y mostrar `result.net` y
   `player.hands[i].outcome` / `net`.
6. **Siguiente ronda:** volver al paso 3 con el saldo de `wallet.balance`.
7. **Fin:** si `wallet.balance` queda por debajo de 10, la partida terminó (`busted`):
   `wallet.maxBet` es 0 y `GET /casual/runs/active` pasa a 404. El usuario también puede
   retirarse con `POST /casual/runs/{id}/retire` (no con una ronda en juego).

### 4. Claves de idempotencia

- Generar un `crypto.randomUUID()` **por intención** (este reparto, esta acción) y
  reutilizarlo **solo** al reintentar esa misma petición (error de red, timeout).
- Repetir la clave devuelve **la misma respuesta** que la primera vez, sin volver a
  ejecutar: no se reparte ni se cobra dos veces.
- Una clave nueva para algo ya hecho sí se ejecuta: no reutilizar claves entre acciones
  distintas.

### 5. Qué mostrar de `RoundView`

- **`dealer.hole` es `null` mientras no se revela.** Dibujar la carta boca abajo.
- **`peekBlackjack: true`:** el dealer tenía blackjack y la ronda terminó en el reparto.
  Conviene animar la revelación antes de mostrar el resultado.
- **`player.active`:** la mano que se está jugando (tras dividir hay varias).
- **`shoeCounts`:** cartas sin ver por rango del motor (0 = As … 9 = diez), con la oculta
  contada como no vista. Es exactamente el `shoe` que espera el motor WASM: las ayudas de
  Casual (EV, recomendaciones) se calculan con él igual que en Práctica.
- **`penetration`:** fracción del zapato ya repartida, para el indicador. El servidor
  rebaraja solo antes de repartir; el front no tiene que avisar nada.
- **`result`:** solo en `settled`. En Casual no trae `review` ni `refilled`.

### 6. Ranking semanal y récords

- **`GET /casual/leaderboard/weekly`:** los 100 mejores picos de las partidas empezadas
  desde el lunes 00:00 UTC (`weekStart`), uno por usuario. Las posiciones empatadas
  comparten `rank`. `me` trae la fila del usuario si jugó esa semana (aunque ya esté en
  `top`, con `isMe: true`). Se puede ver sin sesión.
- **`GET /me/records`:** `casual.runs` (partidas jugadas), `bestPeak` y `mostHands`. Los
  dos últimos faltan si todavía no hay partidas. `ranked` se explica en la fase 4.

### 7. Mensajes sugeridos

| `code` | Mensaje |
|---|---|
| `display_name_required` | «Elige tu nombre antes de jugar: aparece en el ranking.» |
| `run_active` | Recargar la partida activa (no es un error para el usuario). |
| `round_active` | Recargar `GET /play/active` y retomar la ronda. |
| `bet_out_of_range` | «La apuesta debe ser de 10 a 500, en múltiplos de 10.» |
| `insufficient_funds` | «No te alcanza el saldo para esa apuesta.» |
| `action_not_allowed` | Recargar la ronda: el estado local quedó desfasado. |
| `no_active_round` | Recargar: la ronda ya terminó. |
| `run_finished` | «La partida terminó.» y ofrecer una nueva. |

---

## Qué NO hacer

- **No repartir ni liquidar en el navegador.** Las cartas, los resultados y el saldo de
  Casual salen solo del servidor. El front manda intenciones (apostar, pedir…).
- **No calcular `allowed` en el front.** Usar el de la respuesta: tiene en cuenta el saldo
  para doblar y dividir.
- **No guardar el saldo de Casual en `chipsStore`** como si fuera de Práctica: es de la
  partida y lo trae cada respuesta.
- **No reutilizar una clave de idempotencia** para otra acción.
- **No esperar `review` en Casual:** solo viene en Ranked (fase 4).
- **No mostrar emails** en el ranking: solo `displayName`.
- **No editar `contract.ts` a mano.**

## Probar

- Con el backend corriendo (`npm run db:start` y `npm run api`) y una cuenta con nombre
  elegido: crear una partida, repartir, jugar varias rondas y recargar la página a mitad de
  una mano (debe retomarse).
- Repetir a mano un `POST /play/rounds` con la misma `idempotencyKey`: debe devolver la
  misma ronda sin cambiar el saldo.
- Para probar `busted`, apostar todo varias veces hasta perder.
