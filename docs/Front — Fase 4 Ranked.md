# Front — Fase 4: Ranked

**Versión del 2026-10-28 (primera).** Qué cambió respecto de lo que el front conoce:

- Nuevo modo **Ranked**: se juega con las mismas rutas `/play/*` que Casual, con
  `kind: "ranked"`. Endpoints nuevos `GET /ranked/me` y `GET /ranked/leaderboard`.
- `RoundView` de Ranked **no trae `shoeCounts`** y, al liquidar, trae `result.review` (y
  `result.refilled` si hubo recarga). `wallet.refillAvailableAt` aparece cuando el saldo está
  bajo el piso y la recarga todavía no está disponible.
- Tipos nuevos en `src/engine/contract.ts` (generados): `RankedAccountView`,
  `RankedLeaderboard`, `RankedRow`, `Medal`, `SeasonResult`, `RankedRecords`. `MyRecords`
  suma `ranked`.
- Errores nuevos: `refill_pending`. Desaparece el 501 `not_available`.

Se apoya en «Front — Fase 3 Casual.md»: el flujo de una ronda, las claves de idempotencia y
lo que se muestra de `RoundView` son iguales. Aquí solo va lo que cambia. Producto:
secciones 5 y 8.2 de
[Especificación — Cuentas, Casual y Ranked.md](Especificación%20—%20Cuentas,%20Casual%20y%20Ranked.md).
Este documento solo existe en `blackjack-backend/docs/`.

## Estado

| | Backend | Front |
|---|---|---|
| Cuenta de la temporada (`/ranked/me`) | Listo | Por hacer |
| Rondas Ranked (`/play/*` con `kind: "ranked"`) | Listo | Por hacer |
| Revisión al terminar la mano (`result.review`) | Listo | Por hacer |
| Ranking por temporada y medallas | Listo | Por hacer |
| Reto diario, logros, cosméticos (fase 5) | No existe | **No hacer** |

---

## Reglas que el usuario tiene que ver

- **Temporadas mensuales** en UTC (del día 1 a las 00:00 al día 1 siguiente). Cada
  temporada empieza con **500** fichas.
- **Reglas fijas** de Las Vegas: 6 mazos, S17, doblar con cualquier par de cartas, doblar
  tras dividir, hasta 4 manos, sin redividir ases, rendición tardía, peek y 3:2. No se eligen.
- **Apuesta máxima:** el 25 % del saldo en decenas, entre 10 y 500 (con 100 → 20; con
  1.000 → 250). `maxBet` ya viene calculado.
- **Piso de 100:** si el saldo baja de 100, se recarga hasta 100, como mucho una vez cada
  24 h. Las recargas **no suman** al ranking.
- **Ranking por ganancia neta** (`net = saldo − 500 − recargado`), no por saldo. Para
  aparecer y recibir medalla hacen falta **200 manos** en la temporada; antes, la posición
  es «provisional».
- **Sin ayudas durante la mano.** Al terminar, la revisión muestra cada decisión contra la
  óptima.

## Qué añadir

### 1. Endpoints (bajo `/api/v1`)

| Método | Ruta | Cuerpo | Éxito | Errores (`code`) |
|---|---|---|---|---|
| GET | `/ranked/me` | — | 200 `RankedAccountView` | 401 |
| POST | `/play/rounds` | `{ kind: "ranked", bet, idempotencyKey }` | 201 `RoundView` | 400 `bet_out_of_range`, 409 `refill_pending`, 409 `round_active`, 409 `display_name_required` |
| POST | `/play/rounds/{id}/actions` | `{ action, idempotencyKey }` | 200 `RoundView` | Igual que Casual |
| GET | `/play/active` | — | 200 `RoundView` (Casual o Ranked, mirar `kind`) | 404 `no_active_round` |
| GET | `/ranked/leaderboard[?season=AAAA-MM]` | — | 200 `RankedLeaderboard` | 404 `not_found` (temporada que no existe) |
| GET | `/me/records` | — | 200 `MyRecords` (con `ranked`) | — |

- `GET /ranked/me` crea la cuenta de la temporada si es la primera vez y aplica la recarga
  si toca. Llamarlo al entrar al modo y después de cada ronda liquidada no es necesario: el
  `wallet` de cada `RoundView` ya trae saldo, `maxBet` y `refillAvailableAt`.
- En Ranked **no se manda `contextId`** (se ignora): siempre es la cuenta de la temporada
  actual. `RoundView.contextId` es el `id` de esa cuenta.
- El límite por usuario (429 con `Retry-After`) es el mismo de Casual.

### 2. Flujo

1. Si `me.needsDisplayName`, pedir el nombre antes (`display_name_required`).
2. **Al entrar al modo:** `GET /ranked/me` y `GET /play/active` en paralelo.
   - Si hay ronda activa con `kind: "ranked"`, retomarla. Si es `"casual"`, avisar y mandar
     al usuario a terminarla en Casual (repartir en Ranked daría `round_active`).
3. **Apostar:** `POST /play/rounds` con `kind: "ranked"` y una apuesta entre 10 y
   `maxBet`, en múltiplos de 10.
4. **Decidir:** igual que Casual, con los botones de `allowed`. Cada acción puede tardar
   unos cientos de milisegundos más que en Casual (el servidor evalúa la decisión).
   Conviene deshabilitar los botones mientras llega la respuesta.
5. **Liquidación:** animar la oculta y `dealer.hits` como en Casual. Después mostrar
   `result.net`, `result.refilled` (si viene: «Se recargó tu saldo a 100») y la revisión.
6. **Sin saldo:** si `wallet.maxBet` es 0, no se puede apostar hasta
   `wallet.refillAvailableAt` (mostrar la hora local). Al llegar esa hora, `GET /ranked/me`
   devuelve el saldo ya recargado.

### 3. Revisión (`result.review`)

Una entrada por decisión, en orden:

```ts
interface ReviewItem {
  handIndex: number;  // mano (tras dividir hay varias)
  chosen: Action;     // lo que eligió
  best: Action;       // la óptima con lo que veía (la oculta contada como no vista)
  evChosen: number;   // EV en unidades de la apuesta
  evBest: number;
  exact: boolean;     // false si el cálculo de dividir fue aproximado
}
```

- Acierto: `chosen === best`. Si no, el EV perdido es `evBest − evChosen` (en unidades de
  la apuesta; en fichas, multiplicar por `bet`).
- Puede venir vacía: la ronda terminó en el reparto (blackjack o peek) o el usuario no
  decidió nada. Si el cálculo de una decisión falla, esa decisión falta.
- Las decisiones que tomó el servidor al cerrar una ronda abandonada (24 h sin acciones)
  no se revisan.
- No calcular la revisión en el front: el motor WASM no tiene el estado del zapato.

### 4. Lo que **no** viene en Ranked

- `shoeCounts`: sin composición del zapato, **no** se pueden calcular EV ni
  recomendaciones durante la mano. Ocultar el panel de ayudas, la probabilidad de pasarse y
  cualquier conteo calculado. Contar cartas mirando la mesa es legítimo; dárselo calculado,
  no.
- `penetration` sí viene (indicador del zapato).

### 5. Ranking y récords

- **`GET /ranked/leaderboard`** sin `season` es la temporada actual. `top` trae los 100
  primeros con 200 manos o más (empates comparten `rank`). `me` trae la fila del usuario si
  jugó esa temporada: con `provisional: true` si no llega a 200 manos, y entonces `rank` es
  la posición que tendría. Se puede ver sin sesión (sin `me`).
- **Temporadas cerradas** (`closed: true`): las filas traen `medal` (`"gold"`, `"silver"`,
  `"bronze"` o `"top10"`). Una temporada se cierra en la primera hora después de terminar
  (o cuando se liquida su última ronda en juego).
- **`GET /me/records`:** `ranked.seasons` es la lista de temporadas cerradas en las que el
  usuario clasificó (`season`, `rank`, `net`, `hands`, `medal?`), de la más reciente a la
  más antigua; `ranked.bestRank` es su mejor posición final. Vacía si nunca clasificó.

### 6. Mensajes sugeridos

| `code` | Mensaje |
|---|---|
| `bet_out_of_range` | «En Ranked la apuesta es de 10 a {maxBet}, en múltiplos de 10.» |
| `refill_pending` | «Te quedaste sin fichas. La recarga a 100 estará disponible el {refillAvailableAt}.» |
| `round_active` | Recargar `GET /play/active` y retomar la ronda (puede ser de Casual). |
| `display_name_required` | «Elige tu nombre antes de jugar: aparece en el ranking.» |

---

## Qué NO hacer

- **No mostrar ayudas** (EV, recomendación, conteo calculado) en una ronda Ranked, ni pedir
  `shoeCounts` por otra vía.
- **No calcular el ranking ni la ganancia neta** en el front: vienen en `net`.
- **No mandar `contextId`** en Ranked ni guardar el saldo en el navegador.
- **No editar `contract.ts` a mano.**

## Probar

- Con una cuenta con nombre: `GET /ranked/me` (500, `maxBet` 120), jugar varias rondas y
  mirar la revisión al terminar cada una.
- Comprobar que en Ranked no aparece el panel de ayudas y que en Casual sigue.
- Para ver el piso: apostar el máximo hasta bajar de 100 y comprobar `result.refilled`; la
  siguiente vez que baje (antes de 24 h) debe aparecer `refillAvailableAt`.
- El ranking necesita 200 manos para mostrar a alguien en `top`; con menos, la fila propia
  sale como provisional.
