# Front — Salas Casual (multijugador)

**Versión del 2026-10-30 (primera).** Qué cambió respecto de lo que el front conoce:

- Nuevo modo **Salas**: mesas de 5 a 7 asientos contra el dealer, públicas o privadas, en
  tiempo real por **WebSocket**. Hasta 10 espectadores por sala.
- Rutas nuevas: `POST /rooms`, `GET /rooms`, `GET /rooms/{code}` y el WebSocket
  `GET /rooms/{code}/ws`.
- Tipos nuevos en `src/engine/contract.ts` (generados): `CreateRoomRequest`, `RoomInfo`,
  `RoomVisibility`, `RoomState`, `RoomSeat`, `RoomPlayer`, `RoomPhase`,
  `RoomLeaderboardRow`, `TableRoundView`, `SeatRoundView`, `Reaction`, `ClientMessage` y
  `ServerMessage`.

Producto y límites:
[Especificación — Salas Casual.md](Especificación%20—%20Salas%20Casual.md). Este documento
solo existe en `blackjack-backend/docs/`.

## Estado

| | Backend | Front |
|---|---|---|
| Crear, listar y ver salas (HTTP) | Listo | Por hacer |
| Mesa en tiempo real (WebSocket) | Listo | Por hacer |
| Reacciones y ranking interno | Listo | Por hacer |
| Chat de texto | No existe (decisión) | **No hacer** |

---

## Qué añadir

### 0. Proxy de Vite

El WebSocket va por el mismo proxy `/api`, que tiene que dejar pasar el `Upgrade`:

```ts
server: { proxy: { '/api': { target: 'http://127.0.0.1:8080', ws: true } } }
```

URL: `` `${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}/api/v1/rooms/${code}/ws` ``.
La cookie de sesión viaja sola (mismo origen).

### 1. HTTP (bajo `/api/v1`)

| Método | Ruta | Cuerpo | Éxito | Errores (`code`) |
|---|---|---|---|---|
| POST | `/rooms` | `CreateRoomRequest` | 201 `RoomInfo` | 400 `bad_request` (nombre de 3–30, asientos 5–7, preset **o** rules), 422 `engine_error` (reglas inválidas), 409 `room_hosting` (ya tiene una sala), 409 `display_name_required`, 503 `rooms_full`, 429 |
| GET | `/rooms` | — | 200 `RoomInfo[]` (solo públicas; con asientos libres primero) | — |
| GET | `/rooms/{code}` | — | 200 `RoomInfo` (también privadas) | 404 `room_not_found` |
| GET | `/rooms/{code}/ws` | — | WebSocket | 401, 404 `room_not_found`, 403 `room_banned`, 409 `room_full`, 409 `display_name_required` |

- `CreateRoomRequest`: `{ name, visibility: "public" | "private", seats: 5 | 6 | 7, preset?
  | rules?, aids }`. `aids: true` activa las ayudas para todos (ver sección 4).
- El **código** (6 caracteres) es la invitación: `/salas/K7Q2XP` en el front. Los códigos no
  distinguen mayúsculas.
- Los errores del WebSocket antes de conectar llegan como respuesta HTTP: si `new WebSocket`
  falla, pedir `GET /rooms/{code}` para saber si la sala existe, y mostrar un mensaje
  genérico («No se pudo entrar a la sala») si existe.

### 2. WebSocket: mensajes

Todos son JSON con `type`. Los tipos están en `ClientMessage` y `ServerMessage`.

**Del cliente:**

| `type` | Campos | Cuándo |
|---|---|---|
| `sit` | `seat` (1..asientos) | Sentarse en un asiento libre |
| `leaveSeat` | — | Volver a espectador (no con la mano en juego) |
| `bet` | `amount` (10–500, múltiplo de 10, ≤ pila) | En `betting` |
| `action` | `action` (`hit`, `stand`, `double`, `split`, `surrender`) | En su turno |
| `rebuy` | — | Con la pila por debajo de 10 |
| `react` | `reaction` | Siempre (1 cada 2 s) |
| `kick` | `playerId` | Solo el anfitrión |

**Del servidor:**

| `type` | Campos | Qué hacer |
|---|---|---|
| `welcome` | `playerId` | Guardarlo: es «yo» en `seats[].player.id`, `hostId` y el ranking |
| `state` | `room: RoomState` | **Reemplazar** el estado local entero |
| `reaction` | `playerId`, `reaction` | Mostrar la reacción sobre ese jugador |
| `error` | `code`, `message` | Avisar; la conexión sigue (sección 6) |
| `closed` | `reason` (`kicked`, `closed`, `banned`, `full`) | Avisar y volver al lobby; el servidor cierra la conexión |

### 3. Ciclo de la mesa (`room.phase`)

1. `waiting`: nadie sentado. Mostrar asientos libres con «Sentarse».
2. `betting`: los sentados apuestan. `timerMs` es lo que falta; contar hacia atrás desde que
   llegó el mensaje. `seats[i].bet` muestra quién ya apostó. Termina antes si todos apostaron.
3. `playing`: `round` (`TableRoundView`) trae las manos por asiento, el dealer y `turn` (el
   asiento que decide). Si `turn` es mi asiento, mostrar los botones de `round.allowed`; si
   no, deshabilitarlos. `timerMs` es el tiempo del turno (20 s); al vencer, el servidor se
   planta.
4. `results`: la ronda liquidada: `round.dealer` revelado, `round.seats[i].net` y los
   `outcome` de cada mano. Animar la oculta y `dealer.hits` en orden, como en Casual. En 5 s
   vuelve a `betting`.

- **El servidor manda el estado completo** en cada cambio; no hace falta aplicar eventos.
  Para animar, comparar con el estado anterior (cartas nuevas, cambio de turno).
- `seats[i].connected: false`: el jugador se cayó; conserva el asiento 60 s.
- Quien no apuesta 3 rondas seguidas vuelve a espectador.
- **Pila:** `seats[i].stack`. Con menos de 10, ofrecer «Recomprar» (`rebuy`, vuelve a 500).

### 4. Ayudas

- Si `room.info.aids` es `true`, el estado trae `shoeCounts` (cartas sin ver, con la oculta
  como no vista): las ayudas se calculan con el motor WASM igual que en Práctica.
- Si es `false`, **no** viene: ocultar EV, recomendaciones y conteo, como en Ranked.

### 5. Ranking interno, espectadores y anfitrión

- `room.leaderboard`: neto (pila − 500 × compras), manos y compras de todos los que se
  sentaron. **Vive mientras viva la sala**: no se guarda en récords ni en logros.
- `room.spectators`: quienes miran (sin asiento).
- `room.hostId`: el anfitrión. Solo él ve el botón «Echar» junto a cada jugador o
  espectador. Si el anfitrión se va, el rol pasa a otro y llega en el siguiente `state`.
- `seats[i].player.title`: el título equipado, como en los rankings.

### 6. Errores dentro de la sala (`error.code`)

| `code` | Mensaje sugerido |
|---|---|
| `seat_taken` | «Ese asiento ya está ocupado.» |
| `already_seated` | «Ya estás sentado.» |
| `not_seated` | «Siéntate para jugar.» |
| `betting_closed` | «Las apuestas están cerradas.» |
| `bet_out_of_range` | «La apuesta es de 10 a 500, en múltiplos de 10.» |
| `insufficient_funds` | «No te alcanza la pila.» |
| `not_your_turn` | Recargar el estado (no debería pasar si se respeta `turn`). |
| `action_not_allowed` | Igual. |
| `round_in_progress` | «Espera a que termine la ronda.» |
| `rebuy_not_needed` | — |
| `rate_limited` | Ignorar (reacciones demasiado seguidas). |
| `not_host` | — |
| `round_voided` | «Se acabó el zapato: la ronda se anuló y nadie perdió nada.» |
| `bad_request` | Error del front. |

### 7. Reconexión

- Si el WebSocket se corta sin `closed`, reconectar con espera creciente (1 s, 2 s, 4 s…,
  máximo 10 s). Al reconectar llegan `welcome` y un `state` completo: el asiento se conserva
  si fue en menos de 60 s.
- Más de 20 mensajes por segundo cortan la conexión: no mandar mensajes en bucle.
- La sala vive en memoria: si el servidor se reinicia, desaparece. Si al reconectar la
  respuesta es 404, avisar «La sala cerró» y volver al lobby.

### 8. Pantallas sugeridas

- **Lobby:** lista de `GET /rooms` (nombre, anfitrión, asientos ocupados, reglas, ayudas) con
  «Entrar», un campo «Código de sala» y «Crear sala».
- **Crear sala:** nombre, pública/privada, asientos (5–7), reglas (los presets de Casual o
  personalizadas) y ayudas sí/no. Al crear, ir a la sala y mostrar el enlace para copiar.
- **Mesa:** dealer arriba; asientos en arco; panel de apuesta en `betting`; botones de acción
  en mi turno con el temporizador; barra de reacciones; ranking y espectadores a un lado.

---

## Qué NO hacer

- **No repartir ni liquidar en el navegador:** todo sale del `state`.
- **No calcular `allowed`:** usar `round.allowed` (y solo si `turn` es mi asiento).
- **No mostrar ayudas** si `info.aids` es `false`.
- **No hacer chat de texto.**
- **No guardar el ranking de la sala:** muere con ella.
- **No editar `contract.ts` a mano.**

## Probar

- Dos navegadores (o una ventana privada) con cuentas distintas: crear una sala, entrar con
  el código, sentarse los dos, apostar y jugar una ronda; ver los turnos en orden.
- Dejar pasar 20 s en un turno: el servidor se planta.
- Cerrar una pestaña y volver antes de 60 s: el asiento sigue.
- Echar a alguien desde el anfitrión: recibe `closed` con `kicked` y no puede volver.
