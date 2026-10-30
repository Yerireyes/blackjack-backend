# Especificación — Salas Casual (multijugador)

Documento para implementar en el **backend** las salas de Casual: mesas de 5 a 7 asientos
contra el dealer, públicas o privadas, para jugar con amigos en tiempo real. **Única copia:**
`blackjack-backend/docs/`. Lo que le toca al front va en «Front — Salas Casual.md». Estado:
decisiones de producto cerradas (2026-10-29); los valores *(ajustable)* se pueden cambiar
sin rediseñar.

---

## 1. Decisiones

| Tema | Decisión |
|---|---|
| Tiempo real | **WebSocket en nuestro backend** (Axum). Sin servicios externos ni webhooks |
| Fichas | **Pila propia por sala**: 500 al sentarse por primera vez. No toca la partida casual ni Ranked |
| Asientos | El anfitrión elige **5, 6 o 7** al crear la sala |
| Espectadores | Sí, hasta **10** por sala *(ajustable)* |
| Tiempo por decisión | **20 s**; al vencer, el servidor se planta por el jugador |
| Reglas | Las elige el anfitrión al crear la sala (preset o personalizadas, como Casual) |
| Ayudas (EV, recomendaciones) | Las activa o no el anfitrión al crear la sala |
| Comunicación | **Solo reacciones** (emojis de un catálogo fijo); sin chat de texto |
| Ranking | **Interno de la sala**: vive en memoria mientras exista la sala y desaparece con ella |
| Ranked | No se juega en salas |

**Principio rector (igual que Casual):** el servidor baraja con un CSPRNG, reparte, valida
cada acción, juega el dealer y liquida. Los clientes mandan intenciones. La oculta del dealer
y el orden del zapato nunca salen del servidor antes de tiempo.

---

## 2. Arquitectura

- **Todo en memoria.** Una sala es una tarea de Tokio (un «actor») que es dueña de su estado:
  asientos, pilas, zapato, ronda, temporizadores y ranking. Recibe órdenes por un canal y
  manda el estado a cada conexión. No se escribe nada de la sala en Postgres: el ranking
  interno muere con la sala, que es lo pedido. Postgres solo se usa para la sesión y el
  nombre visible.
- **Una sola instancia del backend.** Las salas viven en el proceso. Si el servidor se
  reinicia, las salas desaparecen (los jugadores vuelven al lobby). Con varias instancias
  haría falta repartir las salas (Redis o `LISTEN/NOTIFY`); fuera de alcance.
- **Rutas HTTP** para crear, listar y ver salas; **un WebSocket por sala** para jugar.
- **El reverse proxy** (Caddy/Nginx) tiene que dejar pasar el `Upgrade: websocket` (Caddy lo
  hace solo; en Nginx, `proxy_set_header Upgrade` y `Connection`). El servidor manda un
  ping cada 30 s para que proxies y PaaS no corten la conexión.

### 2.1 Límites

| Límite | Valor | Variable |
|---|---|---|
| Salas abiertas a la vez | **200** | `MAX_ROOMS` |
| Salas que un usuario puede tener como anfitrión | 1 | — |
| Conexiones por sala | 7 asientos + 10 espectadores | — |
| Mensajes por conexión | 20 por segundo (si se pasa, se cierra la conexión) | — |
| Reacciones | 1 cada 2 s por usuario | — |
| Sala sin nadie conectado | se cierra a los 2 min | — |

**Por qué 200:** cada sala tiene como mucho 17 conexiones (3.400 en total) y ocupa menos de
100 KB (zapato, ronda y ranking), unos 20 MB en total. El servidor no calcula nada pesado en
las salas (las ayudas las calcula el navegador con `shoeCounts`), así que el CPU sobra en
un VPS de 1 vCPU. Es un tope conservador: se puede subir con `MAX_ROOMS` cuando haya
métricas.

---

## 3. Salas

- **Código:** 6 caracteres de un alfabeto sin ambiguos (`ABCDEFGHJKMNPQRSTUVWXYZ23456789`).
  Es también el enlace de invitación (`/salas/K7Q2XP` en el front).
- **Pública:** aparece en `GET /rooms`. **Privada:** no aparece; se entra con el código.
- **Crear** exige sesión y nombre elegido (`display_name_required`, como Casual). Al crear se
  eligen: nombre de la sala (3–30 caracteres), pública o privada, asientos (5–7), reglas
  (`preset` o `rules`) y ayudas (sí/no). Nada de eso cambia después.
- **Anfitrión:** quien la crea. Puede **echar** a un jugador o espectador, que no puede volver
  a esa sala. Si el anfitrión se va (sin conexiones durante 60 s), el anfitrión pasa al
  jugador sentado que lleva más tiempo en la sala (o, si no hay, al espectador más antiguo).
- **Cierre:** a los 2 min sin conexiones. Con eso desaparece el ranking interno.

## 4. Jugadores, espectadores y fichas

- **Entrar** (conectar el WebSocket) es entrar como **espectador**. Para jugar se pide un
  asiento libre (`sit`). Se puede volver a espectador (`leaveSeat`) entre rondas.
- **Pila:** 500 fichas la primera vez que el usuario se sienta en esa sala. La pila es del
  usuario en esa sala: si se levanta y se vuelve a sentar, conserva lo que tenía.
- **Recompra:** con menos de 10, `rebuy` vuelve a 500. Cuenta como una compra más.
- **Apuesta:** de 10 a 500, múltiplo de 10 y nunca más que la pila (como Casual). Doblar y
  dividir exigen fichas para otra unidad.
- **Inactividad:** quien está sentado y no apuesta en 3 rondas seguidas pasa a espectador.
- **Desconexión:** un jugador sentado conserva el asiento 60 s. Si le toca jugar mientras
  está desconectado, corre su tiempo y se planta solo. Si no vuelve en 60 s, el asiento se
  libera (su pila queda guardada en la sala).
- **Más de una pestaña:** un usuario puede tener varias conexiones a la misma sala; todas
  controlan el mismo asiento.

## 5. La ronda

Ciclo de la mesa:

1. **Esperando:** nadie sentado. No pasa nada hasta que alguien se sienta.
2. **Apuestas (20 s):** cada jugador sentado apuesta. Termina antes si todos apostaron. Quien
   no apostó no juega esta ronda. Si nadie apostó, se vuelve a esperar.
3. **Reparto:** como en el casino, una carta a cada asiento en orden, la visible del dealer,
   la segunda carta a cada asiento y la oculta. Con peek y blackjack del dealer la ronda
   termina en el reparto.
4. **Turnos (20 s cada decisión):** asiento por asiento, de izquierda a derecha (1 → 7). Cada
   jugador decide con las acciones de `allowed`. Al vencer el tiempo, el servidor se planta.
   Un blackjack natural no juega.
5. **Dealer:** revela y pide según las reglas (solo si alguna mano sigue viva).
6. **Resultados (5 s):** se muestran los netos y se actualiza el ranking. Después vuelve a
   «Apuestas» si hay alguien sentado.

- **Zapato** compartido por la mesa, con las reglas de la sala. Se rebaraja antes de repartir
  al llegar a la penetración o si quedan menos de `12 × jugadores + 10` cartas.
- **Si el zapato se acaba en mitad de una ronda** (posible con un mazo y la mesa llena), la
  ronda se **anula**: se devuelven las apuestas y se rebaraja. Es muy raro y no le cuesta
  nada a nadie.
- **Ayudas:** si el anfitrión las activó, el estado trae `shoeCounts` (cartas sin ver, con la
  oculta contada como no vista), igual que Casual, y el front calcula EV y recomendaciones con
  su motor WASM. Sin ayudas no viene.

## 6. Ranking interno

Por usuario que se sentó alguna vez en la sala: **neto** = pila actual − 500 × compras,
**manos** jugadas y **compras**. Ordenado por neto. Solo existe en memoria; desaparece al
cerrar la sala. No se guarda en récords ni en logros.

## 7. Reacciones

Catálogo fijo: `thumbsUp`, `clap`, `laugh`, `wow`, `sad`, `fire`, `cool`, `gg`. Las mandan
jugadores y espectadores (1 cada 2 s). No hay texto libre, así que no hace falta moderación.

---

## 8. Contrato

Los DTOs se generan con `tsify` en `contract.ts`, como el resto.

### 8.1 HTTP (bajo `/api/v1`)

| Método | Ruta | Qué hace |
|---|---|---|
| POST | `/rooms` | Crea una sala → 201 `RoomInfo`. 409 `room_hosting` (ya tiene una), 503 `rooms_full`, 409 `display_name_required` |
| GET | `/rooms` | Salas públicas → `RoomInfo[]` (las que tienen asientos libres primero) |
| GET | `/rooms/{code}` | Una sala (también privada, con el código) → `RoomInfo`, o 404 `room_not_found` |
| GET | `/rooms/{code}/ws` | WebSocket de la sala (con sesión). 403 `room_banned` si lo echaron, 409 `room_full` si no caben más espectadores |

### 8.2 WebSocket

Mensajes JSON con `type`.

**Del cliente:** `sit { seat }`, `leaveSeat`, `bet { amount }`, `action { action }`,
`rebuy`, `react { reaction }`, `kick { playerId }` (anfitrión).

**Del servidor:**

- `welcome { playerId }`: una vez al conectar; `playerId` identifica al usuario en la sala.
- `state { room }`: el estado completo (`RoomState`) cada vez que cambia algo. El front
  reemplaza el suyo con este, como con `RoundView`.
- `reaction { playerId, reaction }`.
- `error { code, message }`: una orden rechazada (`not_your_turn`, `seat_taken`,
  `bet_out_of_range`, `insufficient_funds`, `action_not_allowed`, `betting_closed`,
  `not_host`, `rate_limited`…). La conexión sigue.
- `closed { reason }`: la sala cerró o echaron al usuario; luego se cierra la conexión.

`RoomState` trae: datos de la sala, asientos (jugador, pila, conectado), espectadores, fase,
milisegundos que le quedan al temporizador, la ronda (`TableRoundView`: manos por asiento,
dealer, de quién es el turno y `allowed` del jugador en turno), `shoeCounts` si hay
ayudas, penetración y el ranking interno.

## 9. Fases de implementación

1. **Motor:** ronda con varios asientos (`engine/src/game/table.rs`), con tests de orden de
   reparto, turnos, peek y liquidación por asiento.
2. **Salas:** actor por sala, límites, rutas HTTP y WebSocket, temporizadores, reconexión,
   reacciones y ranking interno. Tests de integración con clientes WebSocket reales.
3. **Front:** lobby, crear sala, mesa con varios asientos y reacciones («Front — Salas
   Casual.md»).
