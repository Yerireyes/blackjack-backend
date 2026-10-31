# Front — Historial, estadísticas y entrenamiento dirigido

**Versión del 2026-10-31 (primera).** Qué cambió respecto de lo que el front conoce:

- Endpoints nuevos: `GET /me/history` y `GET /me/stats`.
- Tipos nuevos en `src/engine/contract.ts` (generados): `HistoryItem`, `HistoryPage`,
  `ModeStats`, `MyStats`, `DecisionStats`, `KindStats`, `SpotStats`, `DecisionSpot` y
  `SpotKind`. `ReviewItem` suma `spot?: DecisionSpot`.

Producto: sección 8.7 de
[Especificación — Cuentas, Casual y Ranked.md](Especificación%20—%20Cuentas,%20Casual%20y%20Ranked.md).
Este documento solo existe en `blackjack-backend/docs/`.

## Estado

| | Backend | Front |
|---|---|---|
| Historial de rondas | Listo | Por hacer |
| Estadísticas por modo y de decisiones | Listo | Por hacer |
| Entrenamiento dirigido (en Práctica) | Da los puntos débiles | Por hacer (todo en el navegador) |

---

## Qué añadir

### 1. Historial (`GET /me/history`, con sesión)

- Parámetros: `kind` (`casual` | `ranked` | `daily`, opcional), `limit` (1–50, 20 por
  defecto) y `before` (el `nextBefore` de la página anterior). 400 `bad_request` si algo no
  es válido.
- Responde `HistoryPage { items: HistoryItem[], nextBefore? }`. Cada ítem: modo, apuesta,
  neto (seguro incluido), fecha, `player` y `dealer` (con la oculta) como en `RoundView`,
  seguro y, en Ranked y el reto, `review`.
- Lista con scroll infinito: pedir más con `before = nextBefore` hasta que falte.
- Al tocar una ronda, mostrarla como el final de una mano (cartas, resultado y, si hay,
  la revisión).
- Las rondas de las **salas** no están (viven en memoria).

### 2. Estadísticas (`GET /me/stats`, con sesión)

- `modes`: por cada modo jugado, rondas, `net`, `wins`/`pushes`/`losses` (por el neto de
  la ronda), `blackjacks`, `bestWin` y `worstLoss`.
- `decisions` (solo Ranked y el reto: Casual no se revisa):
  - `decisions`, `optimal` y `evLost` (EV perdido total, en unidades de la apuesta).
    Mostrar «% de decisiones óptimas» = `optimal / decisions`.
  - `byKind`: lo mismo para manos duras, suaves y pares.
  - `weakSpots`: hasta 10 situaciones con errores, de la que más EV costó a la que menos.

### 3. `DecisionSpot`

```ts
interface DecisionSpot {
  kind: "hard" | "soft" | "pair";
  total: number;      // total de la mano (As = 11 si no se pasa)
  pair?: number;      // en un par: rango (0 = As … 9 = diez)
  dealerUp: number;   // carta visible del dealer: rango del motor (0 = As … 9 = diez)
}
```

Texto sugerido: «16 duro contra 10», «18 suave contra 9», «par de 8 contra 6».

### 4. Entrenamiento dirigido (en Práctica, sin backend)

1. Pedir `GET /me/stats` y tomar `decisions.weakSpots`.
2. Ofrecer «Practicar mis puntos débiles»: repartir en el navegador manos que caigan en esas
   situaciones (dos cartas que den ese tipo y total, y esa carta visible del dealer).
3. Usar el motor WASM para decir si la decisión fue la óptima, como en Práctica.
4. Repetir las situaciones con más `evLost` más veces (por ejemplo, con un peso
   proporcional).

Sin datos (pocas rondas revisadas), sugerir jugar unas manos de Ranked o el reto diario
primero.

---

## Qué NO hacer

- **No calcular las estadísticas** con el historial: ya vienen agregadas en `/me/stats`.
- **No mandar las manos del entrenamiento al backend**: es Práctica y no se guarda.
- **No editar `contract.ts` a mano.**

## Probar

- Jugar unas rondas de Casual y de Ranked y ver el historial en orden, con paginación.
- En Ranked, cometer un error a propósito (por ejemplo, pedir con 17 contra 10) y verlo en
  `weakSpots`.
