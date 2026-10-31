# Front — Seguro (y dinero par)

**Versión del 2026-10-30 (primera).** Qué cambió respecto de lo que el front conoce:

- **Motor:** `AnalyzeHandResponse.insuranceEv?: number`, solo con un As visible.
- **Mesas del servidor** (Casual, Ranked, reto diario y salas): con un As visible la ronda
  pasa por una fase nueva, `RoundPhase = "insurance"`, antes del peek.
- Tipos que cambian en `src/engine/contract.ts` (generados): `RoundPhase` suma `"insurance"`,
  `AllowedActions` suma `insurance`, `RoundView` suma `insured?`, `RoundResult` suma
  `insuranceNet?`, `SeatRoundView` suma `insured?`, `ClientMessage` suma
  `{ type: "insurance", take }` y hay un tipo nuevo `InsuranceRequest`.

Producto: sección 8.6 de
[Especificación — Cuentas, Casual y Ranked.md](Especificación%20—%20Cuentas,%20Casual%20y%20Ranked.md)
y sección 5 de [Especificación — Salas Casual.md](Especificación%20—%20Salas%20Casual.md).
Este documento solo existe en `blackjack-backend/docs/`.

## Reglas

- Se ofrece cuando el dealer muestra un **As**, después del reparto y antes del peek.
- Cuesta **media apuesta** y paga **2:1** si el dealer tiene blackjack; si no, se pierde.
- **Dinero par:** si el jugador tiene blackjack, tomar el seguro le asegura ganar 1 a 1 haya
  o no blackjack del dealer. Es el mismo seguro con otro nombre: mostrar «¿Dinero par?» en
  lugar de «¿Seguro?» cuando la mano es blackjack.
- Casi nunca conviene (la guía de estrategia lo explica). `insuranceEv` permite mostrarlo.

## Qué añadir

### 1. Ayudas (Práctica, Casual y salas con ayudas)

- Con As visible, `analyzeHand` trae `insuranceEv`: EV por ficha puesta en el seguro
  (`3p − 1`). Negativo → «No conviene»; positivo (zapato cargado de dieces) → «Conviene».
- En Práctica el seguro sigue siendo opcional: el front puede ofrecerlo con este dato.

### 2. Rondas individuales (Casual, Ranked, reto diario)

- `RoundView.phase === "insurance"`: mostrar «¿Seguro?» (o «¿Dinero par?») con dos botones.
  Las acciones de juego (`allowed.hit`…) están todas en `false` hasta decidir, y la oculta
  sigue en `null`.
- `allowed.insurance`: si alcanzan las fichas para la media apuesta. Si es `false`, mostrar
  solo «No».
- Decidir: `POST /play/rounds/{id}/insurance` con `{ take: boolean, idempotencyKey }`.
  Responde el `RoundView` completo:
  - Peek con blackjack: llega ya `settled`, con `peekBlackjack: true`.
  - Si no: `phase: "player"` y se juega normal.
- `RoundView.insured`: `true`/`false` una vez decidido (para mostrar la ficha del seguro).
- Al liquidar, `result.insuranceNet` (si se tomó) es el neto del seguro, **ya sumado** a
  `result.net`. Mostrarlo aparte («Seguro: +20»).
- Errores: 409 `action_not_allowed` (no se ofrece, ya se decidió o no alcanzan las fichas),
  409 `no_active_round`. Mismo límite por usuario y misma idempotencia que las acciones.
- `GET /play/active` puede devolver una ronda en `insurance`: retomarla mostrando la pregunta.

### 3. Salas

- `round.phase === "insurance"` (con `room.phase === "playing"`): **todos** los asientos con
  mano deciden a la vez; no hay `turn`. Mandar `{ type: "insurance", take }`.
- `round.seats[i].insured`: falta mientras ese asiento no decidió; después `true`/`false`.
  Mostrar la pregunta solo a mi asiento si su `insured` falta.
- `timerMs`: 20 s para decidir; al vencer, quien no decidió no lo toma.
- Para asegurar hace falta pila ≥ 1,5 × la apuesta; si no, el servidor responde
  `action_not_allowed`. Conviene mostrar solo «No» en ese caso.
- `round.seats[i].net` en `results` ya incluye el seguro.

---

## Qué NO hacer

- **No liquidar el seguro en el navegador** en las mesas del servidor: viene en la respuesta.
- **No mostrar `insuranceEv` en Ranked ni en el reto diario** (no hay ayudas durante la mano).
- **No editar `contract.ts` a mano.**

## Probar

- Casual: repartir hasta que el dealer muestre un As (o, con la API local y un zapato
  armado, forzarlo). Tomar el seguro con un dealer con blackjack: `result.net` 0 con
  `insuranceNet` igual a la apuesta.
- Con blackjack propio y As del dealer: «¿Dinero par?»; al aceptar, `result.net` = la apuesta.
- Sala: dos jugadores, As del dealer, uno asegura y el otro no; ver los netos en `results`.
