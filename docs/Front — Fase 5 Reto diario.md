# Front — Fase 5: Reto diario

**Versión del 2026-10-29 (primera).** Qué cambió respecto de lo que el front conoce:

- Nuevo modo **Reto diario**: se juega con `/play/*` y `kind: "daily"`. Endpoints nuevos
  `POST /daily/attempts`, `GET /daily/me` y `GET /daily/leaderboard`.
- `RoundKind` suma `"daily"`. `MyRecords` suma `daily`.
- Tipos nuevos en `src/engine/contract.ts` (generados): `DailyAttemptView`,
  `DailyAttemptStatus`, `DailyLeaderboard`, `DailyRow` y `DailyRecords`.
- Errores nuevos: `no_daily_attempt`, `daily_taken` y `daily_finished`.

Se apoya en «Front — Fase 3 Casual.md» (flujo de una ronda, idempotencia, `RoundView`) y en
«Front — Fase 4 Ranked.md» (sin ayudas y revisión al terminar, que funcionan igual aquí).
Producto: secciones 4.2 y 8.3 de
[Especificación — Cuentas, Casual y Ranked.md](Especificación%20—%20Cuentas,%20Casual%20y%20Ranked.md).
Este documento solo existe en `blackjack-backend/docs/`.

## Estado

| | Backend | Front |
|---|---|---|
| Intento del día (`/daily/attempts`, `/daily/me`) | Listo | Por hacer |
| Rondas del reto (`/play/*` con `kind: "daily"`) | Listo | Por hacer |
| Ranking diario y récords | Listo | Por hacer |
| Logros, cosméticos y Facebook | No existe | **No hacer** |

---

## Reglas que el usuario tiene que ver

- **Un reto por día** (UTC: el día cambia a las 00:00 UTC, que en México es la tarde o noche
  del día anterior). Un solo intento por usuario.
- **El mismo zapato para todos ese día**: todos empiezan con las mismas cartas. Las reglas
  son las de Las Vegas.
- **500 fichas y 30 manos.** Apuesta de 10 a 500, en múltiplos de 10 y sin pasar del saldo.
  El intento termina al jugar la mano 30 o si el saldo baja de 10.
- **Gana el saldo final.** Solo entran en el ranking los intentos terminados.
- **Sin ayudas durante la mano**; al terminar cada una, la revisión de decisiones.

## Qué añadir

### 1. Endpoints (bajo `/api/v1`)

| Método | Ruta | Cuerpo | Éxito | Errores (`code`) |
|---|---|---|---|---|
| POST | `/daily/attempts` | — | 201 `DailyAttemptView` | 409 `daily_taken`, 409 `display_name_required` |
| GET | `/daily/me` | — | 200 `DailyAttemptView` (en juego o terminado) | 404 `no_daily_attempt` |
| POST | `/play/rounds` | `{ kind: "daily", bet, idempotencyKey }` | 201 `RoundView` | 404 `no_daily_attempt`, 409 `daily_finished`, 400 `bet_out_of_range`, 409 `insufficient_funds`, 409 `round_active` |
| POST | `/play/rounds/{id}/actions` | `{ action, idempotencyKey }` | 200 `RoundView` | Igual que Casual |
| GET | `/daily/leaderboard[?day=AAAA-MM-DD]` | — | 200 `DailyLeaderboard` | 400 `bad_request` (fecha mal escrita) |
| GET | `/me/records` | — | 200 `MyRecords` (con `daily`) | — |

- `POST /daily/attempts` y el reparto entran en el límite por usuario de siempre (429).
- En el reto **no se manda `contextId`**: siempre es el intento de hoy.

### 2. Flujo

1. Si `me.needsDisplayName`, pedir el nombre antes.
2. **Al entrar al modo:** `GET /daily/me` y `GET /play/active` en paralelo.
   - 404 `no_daily_attempt`: mostrar las reglas y un botón «Empezar el reto de hoy» que
     llama a `POST /daily/attempts`. Avisar que hay **un solo intento**.
   - `status: "active"`: seguir jugando. Si `GET /play/active` trae una ronda con
     `kind: "daily"`, retomarla.
   - `status: "finished"`: mostrar el resultado y el ranking del día, sin botón de jugar.
3. **Jugar:** como Casual, con `kind: "daily"`. Mostrar el contador «mano N de 30»: tras
   cada liquidación, `hands` sube en `GET /daily/me`. Para no pedirlo en cada mano, el front
   puede contar las rondas liquidadas a partir del valor inicial.
4. **Fin:** cuando una ronda liquidada trae `wallet.maxBet: 0`, el intento terminó (mano 30
   o saldo bajo 10). Mostrar el saldo final y la posición (`GET /daily/leaderboard`).
5. **Cambio de día:** un intento sin terminar al llegar las 00:00 UTC queda así: no entra en
   el ranking y no se puede seguir (repartir da `no_daily_attempt`, porque ya es otro día).
   Si había una ronda en juego, sí se puede terminar.

### 3. Qué mostrar de `RoundView`

- Igual que en Ranked: **sin `shoeCounts`** (ocultar EV, recomendaciones y conteo) y con
  `result.review` al liquidar. No viene `refilled` ni `refillAvailableAt`.

### 4. Ranking y récords

- **`GET /daily/leaderboard`** sin `day` es hoy. `top`: los 100 mejores saldos finales de los
  intentos terminados (empates comparten `rank`). `me`: el intento del usuario aunque no haya
  terminado (`finished: false`, con la posición que tendría). Se puede ver sin sesión.
- Con `?day=AAAA-MM-DD` se ven días anteriores. Un día sin intentos devuelve `top: []`.
- **`GET /me/records`:** `daily.finished` (retos terminados), `bestBalance` y `bestRank`
  (mejor posición en un día ya cerrado). Los dos últimos faltan si no hay datos.

### 5. Mensajes sugeridos

| `code` | Mensaje |
|---|---|
| `daily_taken` | «Ya jugaste el reto de hoy. Vuelve mañana.» (y mostrar el resultado) |
| `daily_finished` | «Terminaste el reto de hoy.» |
| `no_daily_attempt` | Mostrar el botón «Empezar el reto de hoy». |

---

## Qué NO hacer

- **No mostrar ayudas** durante la mano.
- **No permitir empezar dos veces:** después de `POST /daily/attempts`, el botón desaparece.
- **No calcular la fecha del reto en hora local:** usar `day` de la respuesta (UTC).
- **No editar `contract.ts` a mano.**

## Probar

- Con dos cuentas con nombre: empezar el reto en las dos y repartir. Las primeras cartas
  deben ser iguales.
- Jugar las 30 manos (o apostar todo hasta bajar de 10): `wallet.maxBet` pasa a 0,
  `GET /daily/me` dice `finished` y el usuario aparece en `GET /daily/leaderboard`.
- Intentar `POST /daily/attempts` otra vez: `daily_taken`.
