# Front — Amigos

**Versión del 2026-10-31 (primera).** Qué cambió respecto de lo que el front conoce:

- Amigos: solicitudes por nombre, lista con quién está conectado y en qué sala pública
  juega, e **invitaciones a salas** (también privadas).
- **Reto entre amigos:** `GET /daily/leaderboard?scope=friends`.
- Tipos nuevos en `src/engine/contract.ts` (generados): `FriendsView`, `FriendView`,
  `FriendRequestView`, `RoomInvitation`, `FriendRequest` e `InviteRequest`.

Producto: sección 8.8 de
[Especificación — Cuentas, Casual y Ranked.md](Especificación%20—%20Cuentas,%20Casual%20y%20Ranked.md).
Este documento solo existe en `blackjack-backend/docs/`.

## Qué añadir

### 1. Endpoints (bajo `/api/v1`, con sesión)

| Método | Ruta | Cuerpo | Éxito | Errores (`code`) |
|---|---|---|---|---|
| GET | `/friends` | — | 200 `FriendsView` | — |
| POST | `/friends/requests` | `{ displayName }` | 202 (solicitud enviada) o 201 (ya eran mutuas: ahora son amigos) | 404 `user_not_found`, 400 (uno mismo), 409 `already_friends`, `request_exists`, `too_many_friends`, `too_many_requests` |
| POST | `/friends/requests/{userId}/accept` | — | 204 | 404 `request_not_found` |
| POST | `/friends/requests/{userId}/decline` | — | 204 (rechaza una recibida o cancela una enviada) | 404 `request_not_found` |
| POST | `/friends/{userId}/remove` | — | 204 | 404 `not_friends` |
| POST | `/rooms/{code}/invite` | `{ userId }` | 204 | 404 `room_not_found`, 409 `not_in_room`, 403 `not_friends` |
| POST | `/me/invitations/{id}/dismiss` | — | 204 | — |
| GET | `/daily/leaderboard?scope=friends` | — | 200 `DailyLeaderboard` | 401 sin sesión |

- Pedir amistad, aceptar, invitar… entran en el límite por usuario (429).
- `userId` es `RoomPlayer.id` de las vistas de amigos.

### 2. `FriendsView`

- `friends`: conectados primero y después por nombre. Cada uno trae `user` (id, nombre y
  título), `online`, `since` y, si está en una **sala pública**, `room` (`RoomInfo`):
  mostrar «Jugando en *Mesa de Ana*» con un botón «Unirme».
- `incoming`: solicitudes recibidas (Aceptar / Rechazar). `outgoing`: las enviadas
  (Cancelar = `decline`).
- `invitations`: invitaciones de los últimos 30 minutos a salas que siguen abiertas, con la
  sala completa. «Unirme» abre la sala por su código; «Descartar» = `dismiss`.

### 3. Avisos sin tiempo real

No hay WebSocket por usuario: pedir `GET /friends` al entrar a la app, al volver a la
pestaña y cada 30 s mientras esté abierta. Mostrar un globo con la cantidad de `incoming`
+ `invitations`.

### 4. Invitar desde una sala

En la mesa, un botón «Invitar» abre la lista de amigos y manda `POST /rooms/{code}/invite`.
Solo funciona estando conectado a la sala (si no, `not_in_room`). Volver a invitar a la
misma persona renueva la invitación.

### 5. Reto entre amigos

En el ranking del reto diario, una pestaña «Amigos» con `?scope=friends`: mismo formato, con
las posiciones contadas solo entre el usuario y sus amigos.

### 6. Mensajes sugeridos

| `code` | Mensaje |
|---|---|
| `user_not_found` | «No hay nadie con ese nombre.» |
| `already_friends` | «Ya son amigos.» |
| `request_exists` | «Ya le mandaste una solicitud.» |
| `too_many_friends` | «Llegaste al máximo de 200 amigos.» |
| `too_many_requests` | «Tienes demasiadas solicitudes pendientes.» |
| `not_in_room` | «Entra a la sala para invitar.» |
| `not_friends` | «Solo puedes invitar a tus amigos.» |

---

## Qué NO hacer

- **No mostrar emails**: solo el nombre visible.
- **No mostrar salas privadas** de un amigo salvo por invitación (el backend no las manda).
- **No editar `contract.ts` a mano.**

## Probar

- Dos cuentas: pedir amistad, aceptar y ver al otro conectado.
- Abrir una sala con una, invitar a la otra y entrar desde la invitación.
- Jugar el reto diario con las dos y ver la pestaña «Amigos» del ranking.
