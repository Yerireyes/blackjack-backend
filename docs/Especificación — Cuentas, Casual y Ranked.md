# Especificación — Cuentas, partidas Casual y Ranked

Documento para implementar en el **backend** (`../Backend`). El front se adapta en paralelo
siguiendo el contrato de la sección 7. **Única copia:** `blackjack-backend/docs/`. El front
la enlaza y no guarda copias; lo que le toca hacer en cada fase va en «Front — Fase N …» en
esta misma carpeta. Estado: decisiones de producto cerradas; los valores
marcados *(ajustable)* son propuestas razonables que se pueden cambiar sin rediseñar.

---

## 1. Resumen

Se agregan cuentas (contraseña + Google) y dos modos con cuenta:

| | **Práctica** (ya existe) | **Casual** | **Ranked** |
|---|---|---|---|
| Cuenta | No | Sí | Sí |
| Quién reparte y liquida | Navegador | **Servidor** | **Servidor** |
| Capital | Libre | 500 por partida | 500 por temporada; piso de 100 |
| Ayudas del motor durante la mano | Sí | Sí | **No** (revisión al terminar la mano) |
| Reglas | Configurables | Elegibles al crear la partida | **Fijas** (Las Vegas) |

Práctica no cambia y no toca el backend.

**Principio rector:** en Casual y Ranked el servidor es la única autoridad. Guarda el zapato,
reparte con un CSPRNG, valida cada acción, juega el dealer y liquida. El cliente nunca recibe
la carta oculta del dealer antes de revelarse ni el orden del zapato. Todo lo que el cliente
envía es una *intención* (apostar X, pedir, plantarse…), nunca un resultado.

---

## 2. Cuentas

*Implementada (fase 1). Migración `api/migrations/20261027000001_accounts.sql`.*

### 2.1 Cambios en `users`

- `display_name text NOT NULL`: nombre visible en rankings.
  - **Formato:** 3–20 caracteres. Se permiten letras latinas (ASCII y Latin‑1: tildes, ñ, ü,
    ç…), dígitos, `_`, `-` y espacios internos simples, con al menos una letra o dígito. Se
    descartan las letras de otros alfabetos (cirílico, griego…) porque algunas son idénticas
    a las latinas.
  - **Normalización:** se recortan los espacios de los extremos y se normaliza a NFC, así
    «José» escrito con la tilde combinada o precompuesta es el mismo nombre.
  - **Unicidad:** sin distinguir mayúsculas, mediante la columna `display_name_key` (minúsculas
    Unicode calculadas por la API). No se usa `lower()` porque con el locale C solo convierte
    ASCII.
  - **Reservados:** `admin`, `dealer`, `casa`, `blackjack`, `crupier`, `soporte`, etc. Se
    comparan sin mayúsculas, tildes, espacios, `_` ni `-`, así que «Cása» o «c-a s_a» también
    se rechazan. La coincidencia tiene que ser exacta: «Casandra» es válido.
- `display_name_set boolean`: `false` mientras el nombre sea provisional. `/auth/me` lo
  expone como `needsDisplayName`. Los usuarios anteriores a esta fase reciben un nombre
  provisional (`Jugador1`, `Jugador2`…) en la migración.
- `password_hash` pasa a ser **nullable**: las cuentas creadas con Google no tienen contraseña.
- El email se conserva (único sin distinguir mayúsculas) y **nunca** se expone en rankings.

### 2.2 Tabla nueva `identities`

```sql
CREATE TABLE identities (
    id          uuid PRIMARY KEY,
    user_id     uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    provider    text NOT NULL CHECK (provider IN ('google')),  -- 'facebook' en una fase futura
    subject     text NOT NULL,          -- "sub" del proveedor; nunca el email
    email       text,
    created_at  timestamptz NOT NULL DEFAULT now(),
    UNIQUE (provider, subject),
    UNIQUE (user_id, provider)          -- una identidad de cada proveedor por usuario
);
```

### 2.3 Google (OAuth 2.0 Authorization Code + PKCE)

**Nunca se vincula una cuenta existente por email de forma automática.** `/auth/register`
no verifica el email, así que cualquiera puede crear una cuenta con contraseña usando un
email ajeno. Si Google se vinculara solo por coincidir el email, el dueño real entraría en
esa cuenta y quien la registró conservaría el acceso con su contraseña. Para vincular,
el usuario entra con su contraseña y lo pide de forma explícita (modo *vincular*).

- `GET /auth/google/start[?link=1]`: genera `state` y `code_verifier` y los guarda en una
  cookie httpOnly **cifrada** (`bj_oauth`) de 10 minutos, restringida a `/api/v1/auth/google`.
  Redirige a Google con `scope=openid email profile`, `code_challenge` S256 y
  `prompt=select_account`. Con `link=1` hace falta una sesión iniciada **y** una prueba de
  vinculación vigente en esa sesión (`POST /auth/google/link-intent`, abajo), que se gasta
  aquí; sin ella redirige con `reauth_required`. El usuario queda guardado en la cookie.
- `POST /auth/google/link-intent` `{ password }` (con sesión): comprueba la contraseña y deja
  en la sesión una prueba de un solo uso que vence a los 5 minutos. 204, o 403
  `invalid_password`, 409 `no_password`, `already_linked` o `google_disabled`.
- `POST /auth/google/unlink` `{ password }` (con sesión): quita Google y revoca las demás
  sesiones del usuario. 200 `Me`, o 403 `invalid_password`, 409 `no_password` (la cuenta se
  quedaría sin forma de entrar) o `not_linked`.
- `GET /auth/google/callback?code&state`:
  1. Lee la cookie y la borra (se usa una sola vez). Comprueba que no haya vencido y que
     `state` coincida.
  2. Intercambia `code` + `code_verifier` en el servidor (nunca en el cliente).
  3. Lee el `id_token` de la respuesta y comprueba `iss`, `aud` (el `GOOGLE_CLIENT_ID`) y `exp`.
     No se verifica la firma: el token llega directo del endpoint de tokens de Google por
     TLS, y OpenID Connect Core 3.1.3.7 lo permite. Así no hace falta descargar las claves
     JWKS.
  4. **Login:** (a) busca por `identities(provider, subject)` e inicia sesión. (b) Si no
     existe y el email ya tiene cuenta, responde `account_exists` sin vincular. (c) Si el email
     no está verificado, responde `email_unverified`. (d) Si no, crea un usuario sin contraseña
     con un `display_name` provisional único (`Jugador4821`) y `needsDisplayName = true`.
  5. **Vincular:** la sesión del callback tiene que ser la misma que inició el flujo. Agrega
     la identidad al usuario. Si ya era suya, no hace nada. Responde `identity_in_use` si la
     tiene otra cuenta y `already_linked` si el usuario ya tiene otra cuenta de Google.
  6. En un login emite el JWT en `bj_session`. Siempre redirige a `FRONTEND_ORIGIN/` con
     `?login=ok`, `?link=ok` o `?login=error&code=...` / `?link=error&code=...`.
- **Códigos de error en la redirección:** `google_disabled`, `invalid_state`, `cancelled`
  (el usuario canceló en Google), `google_error`, `account_exists`, `email_unverified`,
  `unauthorized`, `reauth_required`, `identity_in_use`, `already_linked` e `internal`.
- **Variables:** `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET` y `GOOGLE_REDIRECT_URI`.
  - Google queda habilitado si están el ID y el secreto; entonces la URI es obligatoria.
  - Sin ellos el servidor arranca igual, con Google deshabilitado, y `/auth/providers`
    devuelve `{ google: false }` para que el front oculte el botón.
  - Si solo hay uno de los dos, el servidor no arranca (es un error de configuración).
- **Desarrollo:** el front accede a la API a través del proxy de Vite (`/api` →
  `127.0.0.1:8080`), así que todo comparte el origen `http://localhost:5180`. La URI de
  redirección es `http://localhost:5180/api/v1/auth/google/callback`.

### 2.4 Endpoints de cuenta (todos bajo `/api/v1`)

| Método | Ruta | Cambio |
|---|---|---|
| POST | `/auth/register` | Pide también `displayName`; 409 `email_taken` o `display_name_taken` |
| POST | `/auth/login` | Si la cuenta no tiene contraseña, el mismo error genérico |
| GET | `/auth/providers` | `{ google: boolean }` |
| GET | `/auth/google/start[?link=1]` · `/auth/google/callback` | Nuevos (2.3) |
| POST | `/auth/google/link-intent` · `/auth/google/unlink` | Nuevos (2.3), con `{ password }` |
| POST | `/auth/logout-all` | Revoca todas las sesiones del usuario |
| GET | `/auth/me` | Ver `Me` abajo |
| PATCH | `/me` | `{ displayName }` con la validación de 2.1; confirma el nombre provisional |

`register`, `login`, `/auth/me` y `PATCH /me` devuelven el mismo objeto (nunca el hash):

```ts
interface Me {
  id: string; email: string; displayName: string;
  needsDisplayName: boolean; hasPassword: boolean;
  providers: string[];   // proveedores vinculados: ["google"]
  createdAt: string;
}
```

- Un nombre inválido responde 400 `display_name_invalid`, con el motivo en `message`.
- El registro, el login, las rutas de Google y `PATCH /me` comparten el límite de tasa
  por IP: ráfaga de `AUTH_BURST` (10) y después una petición cada 6 s.
- Una contraseña incorrecta en una acción con sesión es 403 `invalid_password`, nunca 401:
  para el front, un 401 con sesión significa que la sesión se cerró.
- Front y API comparten origen (proxy de Vite en desarrollo, reverse proxy en producción),
  así que el front no depende de CORS. CORS sigue restringido a `FRONTEND_ORIGIN` y permite
  `PATCH`, solo como resguardo.

**Fuera de alcance de esta fase:** recuperación de contraseña por email (requiere un servicio
de correo), cambiar la contraseña o crear una en cuentas de Google, Facebook y borrar la
cuenta.

---

## 3. Motor de juego en el servidor (base de Casual y Ranked)

Un módulo puro, sin IO (`engine/src/game/` o un crate `game`), testeable sin base de datos.
Debe replicar exactamente las reglas que hoy aplica el front en
`../Frontend/src/store/tableStore.ts` (`allowedActions`, `settleHand`, `dealerMustHit`, el orden
de reparto, el peek y la división de ases). Las reglas son del motor y no cambian. Los casos de
`tableStore.test.ts` sirven como especificación ejecutable: hay que portarlos.

### 3.1 Estado

```text
Shoe        { cards: Vec<Card>, next: usize, decks, penetration }   // orden barajado; solo servidor
RoundState  { bet, player_hands: Vec<Hand>, active, dealer: { up, hole, hole_revealed, hits },
              phase: Player | Dealer | Settled, decisions: Vec<Decision> }
Hand        { cards, from_split, split_aces, doubled, surrendered, done, outcome, net }
Decision    { hand_index, action, ev_chosen, ev_best, best, exact }  // para la revisión (sección 5.4)
```

- **Barajado:** Fisher–Yates con un CSPRNG (`rand::rngs::OsRng` o ChaCha20 sembrado desde
  `OsRng`). Se rebaraja al alcanzar la penetración, antes de repartir la ronda siguiente.
- **Sin bots** en Casual ni Ranked: la mesa es el jugador contra el dealer.
- **Peek:** si el dealer tiene blackjack con peek, la ronda termina en el reparto. La respuesta
  lo indica (`peekBlackjack: true`) y el front anima la revelación. El servidor ya liquidó.

### 3.2 Liquidación y fichas

- Todos los importes son **enteros** (`bigint`). Las apuestas son múltiplos de 10, así que 3:2
  paga 15 y rendirse devuelve 5 por cada 10. Se calcula `round(net × bet)` igual que el front.
- **Doblar y dividir** solo se permiten si el saldo cubre la unidad extra:
  `saldo − bet × unidades_en_juego ≥ bet`. Esa condición se suma a `allowedActions`.
- La apuesta no se descuenta al repartir. Se aplica el neto al liquidar, en la misma
  transacción que cierra la ronda.

### 3.3 Concurrencia, idempotencia y auditoría

- **Una sola ronda activa por usuario:**
  `CREATE UNIQUE INDEX ... ON rounds (user_id) WHERE status = 'active'`.
- Cada operación corre en una transacción con `SELECT ... FOR UPDATE` sobre la fila de la
  cartera (la partida casual o la cuenta ranked) y sobre la ronda.
- **Idempotencia:** toda petición que cambia estado lleva `idempotencyKey` (uuid del cliente).
  `UNIQUE (round_id, idempotency_key)` en las acciones. Si una clave se repite, se devuelve la
  misma respuesta sin volver a ejecutar.
- **Retomar:** si el cliente se desconecta, la ronda sigue activa y `GET /play/active` la
  devuelve. *(Ajustable)* Una ronda sin acciones durante 24 h se cierra con «plantarse» en
  todas las manos pendientes.
- Las rondas liquidadas se conservan con su estado final completo (auditoría y revisión).
- Límite de tasa por usuario en las acciones *(ajustable: 5/s, ráfaga de 10)*.

---

## 4. Casual

### 4.1 Reglas

- Cada **partida** empieza con **500** fichas, con las reglas que elija el usuario: un preset
  (`vegas`, `europe`, `sixToFive`) o reglas personalizadas validadas como en `/analyze/*`.
  Con 6:5 la apuesta mínima es 10 y el pago se redondea con `round()`.
- Apuesta: mínimo 10, máximo 500 y nunca más que el saldo, en múltiplos de 10.
- La partida **termina** cuando el usuario se retira (`POST /casual/runs/{id}/retire`) o
  cuando el saldo queda por debajo de 10. Una partida terminada no se reabre. Se puede tener
  **una sola partida casual activa** a la vez.
- **Ayudas:** en Casual el cliente recibe la composición restante del zapato (`shoeCounts`,
  con la oculta contada como no vista), así el front sigue calculando EV y recomendaciones con
  su motor WASM igual que hoy. El orden del zapato nunca se envía.

### 4.2 Récords e incentivos (para que Casual tenga motivos para volver)

Fase 3 (núcleo):

- Por partida se guardan `peak` (saldo máximo alcanzado), `hands`, `final_balance`, `ended_reason`.
- **Récords personales:** mejor pico, más manos sobrevividas y mejor racha de decisiones óptimas.
- **Ranking casual semanal** por mejor pico (lunes 00:00 UTC). Es confiable porque el servidor
  reparte.

Fase 5 (extras, solo para dimensionar el modelo):

- **Reto diario (hecho, ver 8.3):** mismas reglas y mismo zapato para todos ese día. Semilla:
  `HMAC(DAILY_SEED_SECRET, fecha)`, secreta para que no se pueda precalcular. Un intento por
  usuario y día, 30 manos; ranking diario por saldo final.
- **Logros** (**hechos**, ver 8.4: «10 decisiones óptimas seguidas», «sobrevive 50 manos con
  6:5»…) y **recompensas cosméticas** (paño, dorsos, fichas, títulos; **hechas**, ver 8.5). **Nunca**
  se dan fichas de Ranked.

---

## 5. Ranked

### 5.1 Reglas fijas

Las Vegas para todos: 6 mazos, S17, doblar con cualquier par de cartas, DAS, hasta 4 manos, sin
redividir ases, rendición tardía, peek, 3:2, penetración 75 %. Viven en código, no en la
petición.

### 5.2 Temporadas

- **Mensuales**, en UTC: desde el día 1 a las 00:00 hasta el día 1 del mes siguiente.
- Al jugar por primera vez en la temporada se crea la cuenta ranked del usuario con **500**.
- Al cerrar la temporada se guarda la posición final y se asignan medallas:
  1.º, 2.º, 3.º y top 10 %, solo entre quienes cumplan el mínimo de manos.
  Una ronda activa al cambiar la temporada se liquida en la temporada en que empezó.

### 5.3 Saldo, piso y límites anti-abuso

Combinación decidida (**todas** aplican):

1. **Piso de 100 con recarga limitada.** Cuando, al liquidar, el saldo queda por debajo de
   100, se recarga hasta 100 **solo si** pasaron 24 h o más desde la última recarga. Si no, se
   sigue jugando con lo que haya (si queda en 10 o más). Con menos de 10 no se puede apostar
   hasta que la recarga esté disponible. La respuesta incluye `refillAvailableAt`.
   Cada recarga se registra (`ranked_refills`).
2. **El ranking se ordena por ganancia neta**, no por saldo:
   `net = balance − 500 − total_recargado`. Recargar permite seguir jugando, pero no da puntos.
3. **Apuesta máxima relativa:** `max_bet = clamp(floor(0.25 × saldo / 10) × 10, 10, 500)`.
   Ejemplos: con 100 → 20; con 1.000 → 250; con 4.000 o más → 500.
4. **Mínimo de 200 manos** en la temporada para aparecer en el ranking y recibir medallas.
   Debajo de eso, el usuario ve su posición como «provisional».

### 5.4 Sin ayudas durante la mano, revisión al terminar

- **Durante la ronda** el cliente **no** recibe `shoeCounts` ni ninguna recomendación. Solo las
  cartas visibles, las acciones permitidas y su saldo y límites.
- **En cada decisión** el servidor llama a `analyze_hand` del motor con la información que tenía
  el jugador: cartas visibles, la oculta contada como no vista y la composición restante. Guarda
  `best`, `ev_best`, `ev_chosen` y `exact` en `Decision`. Cuesta unos 55–300 ms por decisión;
  se ejecuta en `spawn_blocking`, como en `/analyze/*`.
- **Al liquidar**, la respuesta incluye `review`: por decisión, la acción elegida, la óptima y el
  EV perdido. Así el modo sigue siendo didáctico sin dar ventaja durante la mano.
- Contar cartas mirando la mesa es habilidad legítima; lo que no se hace es dárselo calculado.

---

## 6. Modelo de datos (resumen)

```text
users            + display_name (único ci), password_hash NULL
identities       (2.2)
casual_runs      id, user_id, rules jsonb, shoe jsonb, balance, peak, hands, status
                 (active|retired|busted), started_at, ended_at
                 UNIQUE (user_id) WHERE status = 'active'
seasons          id, starts_at, ends_at, closed_at
ranked_accounts  (user_id, season_id) PK, shoe jsonb, balance, refilled_total, last_refill_at,
                 hands, peak
ranked_refills   id, account_id (= usuario + temporada), amount, at
season_results   season_id, user_id, rank, net, hands, medal
rounds           id, user_id, kind (casual|ranked), context_id, bet, state jsonb, status
                 (active|settled), net, created_at, settled_at
                 UNIQUE (user_id) WHERE status = 'active'
round_actions    id, round_id, seq, action, idempotency_key, created_at
                 UNIQUE (round_id, idempotency_key)
```

Las tablas `game_sessions` y `hands` actuales quedan como están (estadísticas del modo práctica
con sesión iniciada). La revisión de Casual y Ranked sale de `rounds.state.decisions`.

---

## 7. Contrato HTTP de juego (para el front)

**Los DTOs de Casual y Ranked sí se generan; el front no los copia a mano.** `RoundView`,
los cuerpos de los endpoints de 7.1, las filas de los rankings y los récords se definen en
Rust con `tsify` (junto a los de `engine/src/dto.rs`), así que `npm run gen:contract` los
escribe en `contract/contract.ts` y en `../Frontend/src/engine/contract.ts`, igual que los de
`/analyze/*`. `npm run check:contract` falla si quedan desfasados.

Los tipos de cuentas (`Me`, `Providers`, `ApiError`, sección 2.4) son la excepción: no pasan
por el motor y el front los declara a mano, como indica «Front — Fase 1 Cuentas.md».

### 7.1 Endpoints

| Método | Ruta | Qué hace |
|---|---|---|
| POST | `/casual/runs` | `{ rules \| preset }` → crea la partida (409 si ya hay una activa) |
| GET | `/casual/runs/active` | La partida activa o 404 |
| POST | `/casual/runs/{id}/retire` | Cierra la partida (409 si hay una ronda activa) |
| GET | `/ranked/me` | Cuenta ranked de la temporada actual (la crea si no existe) |
| POST | `/play/rounds` | `{ kind, contextId?, bet, idempotencyKey }` → reparte |
| GET | `/play/active` | Ronda activa, si la hay, como `RoundView` |
| POST | `/play/rounds/{id}/actions` | `{ action, idempotencyKey }` → `RoundView` |
| GET | `/ranked/leaderboard?season=` | Top 100 + la fila del usuario |
| GET | `/casual/leaderboard/weekly` | Top 100 por pico de la semana |
| GET | `/me/records` | Récords personales y medallas |
| POST | `/daily/attempts` | Empieza el reto de hoy (409 si ya lo empezó) |
| GET | `/daily/me` | El intento de hoy o 404 |
| GET | `/daily/leaderboard?day=` | Top 100 del día por saldo final + la fila del usuario |
| GET | `/me/achievements` | Logros desbloqueados y racha de decisiones óptimas |
| GET | `/cosmetics` | Catálogo de cosméticos (público) |
| GET · PUT | `/me/cosmetics` | Los que tiene y los que tiene puestos; `PUT` los cambia |

### 7.2 `RoundView` (lo único que el cliente ve de una ronda)

```ts
interface RoundView {
  id: string;
  kind: 'casual' | 'ranked';
  contextId: string;              // la partida casual (o la cuenta ranked)
  bet: number;
  phase: 'player' | 'settled';    // el turno del dealer se resuelve dentro de la misma respuesta
  player: { hands: HandView[]; active: number };
  dealer: {
    up: CardView;
    hole: CardView | null;        // null mientras no se revela
    hits: CardView[];             // en orden, para que el front las anime una a una
  };
  allowed: { hit: boolean; stand: boolean; double: boolean; split: boolean; surrender: boolean };
  peekBlackjack: boolean;
  wallet: { balance: number; maxBet: number; refillAvailableAt?: string };
  shoeCounts?: number[];          // solo casual: composición restante (oculta contada como no vista)
  penetration: number;            // fracción repartida, para el indicador del zapato
  result?: {                      // solo en 'settled'
    net: number;                  // fichas, entero
    refilled?: number;            // ranked: recarga aplicada en esta liquidación
    review?: ReviewItem[];        // ranked (y casual si se quiere)
  };
}
interface CardView { rank: 'A'|'2'|'3'|'4'|'5'|'6'|'7'|'8'|'9'|'10'|'J'|'Q'|'K'; suit: 'S'|'H'|'D'|'C' }
interface HandView { cards: CardView[]; doubled: boolean; surrendered: boolean; fromSplit: boolean;
                     outcome?: 'blackjack'|'win'|'push'|'lose'|'bust'|'surrender'; net?: number }
interface ReviewItem { handIndex: number; chosen: Action; best: Action; evChosen: number;
                       evBest: number; exact: boolean }
```

Errores con el formato de siempre (`{ code, message }`). Códigos nuevos:
`round_active`, `no_active_round`, `bet_out_of_range`, `insufficient_funds`,
`action_not_allowed`, `run_finished`, `refill_pending`, `display_name_taken`,
`display_name_invalid`. Añadidos al implementar Casual: `run_active`, `no_active_run` y
`display_name_required` (`not_available` dejó de existir con la fase 4). Del reto diario:
`no_daily_attempt`, `daily_taken` y `daily_finished`. De los cosméticos:
`cosmetic_locked` y `cosmetic_wrong_slot`. El detalle por endpoint está en «Front — Fase 3
Casual.md», «Front — Fase 4 Ranked.md» y los tres «Front — Fase 5 …» (reto diario, logros
y cosméticos).

---

## 8. Fases y criterios de aceptación

1. **Cuentas (hecha).** Migración de `users` + `identities`, `display_name`, Google OAuth y los
   endpoints de 2.4. *Tests:* registro con nombre duplicado → 409; callback con `state`
   inválido o sin cookie → error; Google con el email de una cuenta existente **no** se
   vincula solo (`account_exists`) y sí se vincula de forma explícita desde la sesión;
   `/auth/me` nunca devuelve el hash. Se prueban contra un endpoint de tokens de Google
   simulado, que también comprueba PKCE.
2. **Motor de juego (hecha).** Módulo puro de la sección 3 con los casos de `tableStore.test.ts`
   portados, más propiedades: el zapato nunca reparte una carta dos veces; el neto siempre es
   entero; la oculta nunca aparece en un `RoundView` sin revelar. Está en `engine/src/game/`
   (ver 8.1).
3. **Casual (hecha).** Tablas, endpoints de 7.1, récords y ranking semanal. *Tests:* dos rondas activas
   → 409; misma `idempotencyKey` → misma respuesta y un solo descuento; retirarse con una ronda
   activa → 409; con saldo < 10 la partida queda `busted`. Decisiones en 8.1.
4. **Ranked (hecha).** Temporadas, piso, recarga cada 24 h, apuesta máxima relativa, ganancia neta,
   mínimo de manos, revisión. *Tests:* recarga solo una vez en 24 h; `max_bet` según la
   fórmula; el ranking excluye a quien tenga menos de 200 manos; sin `shoeCounts` en ranked;
   `review` coincide con `analyze_hand` para el mismo estado. Decisiones en 8.2.
5. **Extras.** Reto diario (**hecho**, 8.3), logros (**hechos**, 8.4), cosméticos
   (**hechos**, 8.5) y Facebook (pendiente).

### 8.1 Cómo quedaron las fases 2 y 3

**Motor (`engine/src/game/`).** `GameShoe` (orden real, Fisher–Yates con el generador que
le pasa la API: `rand::rng()`, ChaCha sembrado desde el SO), `Round` (reparto, `allowed`,
`act`, turno del dealer y liquidación) y los DTOs de 7.2 en `contract.rs`. Se rebaraja antes
de repartir si se alcanzó la penetración o si quedan menos cartas de las que necesita la peor
ronda (`60 + 31 × maxSplitHands + 26` puntos), así el motor del front siempre puede calcular.
El neto en fichas es `round(neto × apuesta)` como `Math.round` de JavaScript.

**API.** Decisiones que completan o ajustan la especificación:

- **Bloqueo por usuario.** Cada operación de juego (crear o cerrar partida, repartir, actuar)
  empieza con `SELECT … FROM users … FOR UPDATE`, y después bloquea la partida y la ronda. Así
  una clave de idempotencia repetida en paralelo espera y devuelve la respuesta guardada.
- **Idempotencia.** El reparto guarda su respuesta en `rounds.response` con
  `UNIQUE (user_id, idempotency_key)`; cada acción, en `round_actions.response` con
  `UNIQUE (round_id, idempotency_key)`. Repetir la clave devuelve **la misma respuesta**,
  aunque la ronda haya avanzado después.
- **Nombre obligatorio.** `POST /casual/runs` responde 409 `display_name_required` si el
  nombre sigue siendo provisional: el nombre sale en el ranking.
- **`contextId` opcional** en `POST /play/rounds`: sin él se usa la partida activa.
- **Ranking semanal:** el mejor pico por usuario entre las partidas **empezadas** esa semana
  (desde el lunes 00:00 UTC). Es público: sin sesión se ve sin la fila propia.
- **Límite de tasa por usuario** en `POST /casual/runs`, `/casual/runs/{id}/retire`,
  `/play/rounds` y `/play/rounds/{id}/actions`: 5 por segundo con ráfaga `PLAY_BURST` (10). La
  clave es el usuario del JWT (sin consultar la base) o, sin sesión, la IP.
- **Casual sin revisión.** `result.review` no viene en Casual: el front ya tiene
  `shoeCounts` y su motor. El cálculo por decisión llega con Ranked (fase 4).

**Pendiente de Casual:**

- Récord «mejor racha de decisiones óptimas»: existe desde los logros (8.4), pero cuenta
  solo las decisiones revisadas de Ranked y del reto diario. En Casual no se evalúan para no
  gastar 55–300 ms de CPU en cada acción; se puede activar si hace falta.
- ~~Cierre automático de rondas sin acciones durante 24 h~~: hecho en la fase 4 (8.2).

### 8.2 Cómo quedó la fase 4 (Ranked)

Migración `api/migrations/20261030000001_ranked.sql`; rutas en `api/src/routes/ranked.rs`
y `api/src/routes/play.rs`.

- **Temporadas.** Id `"AAAA-MM"` (UTC). La fila se crea la primera vez que alguien usa la
  temporada. La cuenta del usuario (`ranked_accounts`, una por temporada) se crea con 500
  en `GET /ranked/me` o en el primer reparto; su `id` es el `contextId` de las rondas.
- **Reparto.** `POST /play/rounds` con `kind: "ranked"` siempre usa la cuenta de la
  temporada actual (`contextId` se ignora) y exige nombre elegido (`display_name_required`,
  como Casual: el nombre sale en el ranking). Reglas de Las Vegas en código.
- **Piso y recarga.** Al liquidar, si el saldo queda bajo 100 y nunca se recargó o pasaron
  24 h, se recarga hasta 100 (`result.refilled`). Si no se puede, se sigue con lo que hay y
  `wallet.refillAvailableAt` dice cuándo. La recarga pendiente también se aplica sola al
  volver (`GET /ranked/me` o el siguiente reparto). Con menos de 10 y sin recarga
  disponible, repartir da 409 `refill_pending`. Cada recarga queda en `ranked_refills`.
- **Ganancia neta.** Columna generada `net = balance − 500 − refilled_total`, indexada por
  temporada.
- **Apuesta.** De 10 a `maxBet = clamp(⌊0,25 × saldo / 10⌋ × 10, 10, 500)`, múltiplo de 10;
  fuera de eso, 400 `bet_out_of_range`. Doblar y dividir siguen exigiendo saldo para otra
  unidad, como en Casual.
- **Sin ayudas.** `RoundView` de Ranked no trae `shoeCounts`.
- **Revisión.** Antes de aplicar cada acción, el servidor llama a `analyze_hand` en
  `spawn_blocking` (límite de 5 s) con las cartas del jugador, la visible del dealer y lo no
  visto (oculta incluida), y guarda el `ReviewItem` en `rounds.state.decisions`. Si el
  cálculo falla, la acción sigue y esa decisión queda sin revisar. Al liquidar,
  `result.review` trae todas. El reparto no tiene decisiones (un blackjack o un peek dan
  `review: []`).
- **Ranking** (`GET /ranked/leaderboard?season=`). Top 100 por `net` entre quienes tienen
  200 manos o más; empates comparten posición. `me` trae la fila del usuario; si no llega a
  200 manos, `provisional: true` y `rank` es la posición que tendría. Público; sin `season`
  es la actual; una temporada que no existe da 404.
- **Cierre de temporada.** Una tarea horaria (`play::housekeeping`) cierra las temporadas
  terminadas que no tengan rondas en juego: guarda en `season_results` la posición, la
  ganancia y las manos de quienes tienen el mínimo, con medalla `gold`, `silver`, `bronze`
  (posiciones 1–3, los empates comparten) o `top10` (posición ≤ ⌈10 % de los clasificados⌉).
  `GET /me/records` devuelve `ranked.seasons` y `bestRank`.
- **Rondas abandonadas.** La misma tarea cierra las rondas (Casual o Ranked) sin acciones
  durante 24 h, plantándose en las manos pendientes. Esas decisiones automáticas no se
  revisan. Así una temporada nunca queda abierta para siempre por una ronda olvidada.

### 8.3 Cómo quedó el reto diario (fase 5)

Migración `api/migrations/20261031000001_daily.sql`; rutas en `api/src/routes/daily.rs` y
`api/src/routes/play.rs` (`kind: "daily"`).

- **Día** en UTC (`"AAAA-MM-DD"`). Reglas de Las Vegas en código, 500 fichas, 30 manos.
- **Zapato.** `ChaCha20Rng` sembrado con `HMAC-SHA256(clave, "AAAA-MM-DD:n")`, donde `n` es
  el número de barajada del intento (0 al empezar). Todos los que empiezan ese día reciben el
  mismo orden; si alguien llega a rebarajar, la barajada `n` también es la misma para todos.
  Después de la primera decisión distinta, cada jugador ve otras cartas (las mismas, pero
  corridas), como en una mesa real.
- **Clave.** `DAILY_SEED_SECRET` (32 caracteres o más). Sin ella se deriva de `JWT_SECRET`
  (SHA-512 con un prefijo propio). Fijarla en producción permite rotar `JWT_SECRET` sin
  cambiar el zapato del día.
- **Intento.** `POST /daily/attempts` crea el de hoy (uno por usuario y día: 409
  `daily_taken`) y exige nombre elegido. Se reparte con `POST /play/rounds` y
  `kind: "daily"`, siempre con el intento de hoy (`contextId` se ignora): sin intento, 404
  `no_daily_attempt`; terminado, 409 `daily_finished`. Apuesta como Casual: 10–500, múltiplo
  de 10 y no más que el saldo. Termina a las 30 manos o con menos de 10.
- **Sin ayudas y con revisión**, como Ranked: sin `shoeCounts` y con `result.review`.
- **Un intento que no se terminó ese día** queda sin terminar: no entra en el ranking y no
  se puede seguir otro día (la ronda en juego, si la hay, sí se puede terminar o se cierra
  sola a las 24 h).
- **Ranking** (`GET /daily/leaderboard?day=`). Top 100 de intentos **terminados** por saldo
  final (empates comparten posición; desempata quien terminó antes). `me` trae el intento del
  usuario aunque no haya terminado (`finished: false`, con la posición que tendría). Público.
- **Récords.** `MyRecords.daily`: retos terminados, mejor saldo final y mejor posición en
  días ya cerrados (antes de hoy).
- **Riesgo conocido.** Como el zapato es el mismo para todos, alguien con una segunda cuenta
  puede jugar primero y anotar el orden de las primeras cartas. Lo limitan el límite de tasa
  del registro y que el orden diverge tras la primera decisión distinta; si importa, la
  alternativa es un zapato por usuario (mismo día, misma dificultad media) a cambio de perder
  «el mismo reto para todos».

### 8.4 Cómo quedaron los logros (fase 5)

Migración `api/migrations/20261101000001_achievements.sql`; lógica en
`api/src/routes/achievements.rs`.

- **Catálogo en código** (`AchievementId` en el contrato). Los textos los pone el front; las
  condiciones son estas:

  | Id | Condición |
  |---|---|
  | `firstWin` | Ganar una ronda (neto > 0) en cualquier modo del servidor |
  | `blackjack` | Sacar un blackjack natural |
  | `perfectSplit` | Dividir y ganar todas las manos de la ronda |
  | `optimal10` / `optimal50` | 10 / 50 decisiones óptimas seguidas (Ranked y reto diario) |
  | `sixToFiveSurvivor` | Llegar a 50 manos en una partida casual que paga menos de 3:2 |
  | `doubleUp` | Llegar a 1.000 fichas en una partida casual |
  | `rankedQualified` | Llegar a 200 manos en una temporada ranked |
  | `rankedMedal` | Recibir medalla al cerrar una temporada ranked |
  | `dailyComplete` | Jugar las 30 manos del reto diario |

- **Cuándo.** Al liquidar cada ronda, dentro de la misma transacción (también si la liquida
  el cierre de rondas abandonadas). Los nuevos vienen en `result.unlocked` y quedan en la
  respuesta guardada para la idempotencia. `rankedMedal` se otorga al cerrar la temporada.
- **Una vez.** `user_achievements (user_id, achievement)` es la clave: un logro ya obtenido
  no se vuelve a anunciar.
- **Racha.** `player_stats` guarda la racha actual y la mejor. Cuenta solo las decisiones
  revisadas (Ranked y reto diario): una óptima suma, cualquier otra vuelve a 0. Las rondas
  sin decisiones (blackjack, peek) no la cambian. `GET /me/achievements` la devuelve.
- **Recompensas:** los cosméticos de 8.5; nunca dan fichas de Ranked.

### 8.5 Cómo quedaron los cosméticos (fase 5)

Migración `api/migrations/20261102000001_cosmetics.sql`; lógica en
`api/src/routes/cosmetics.rs`.

- **Solo apariencia.** Paño, dorso de las cartas, fichas y título. No dan fichas, saldo ni
  ventaja; el front los dibuja.
- **Catálogo en código** (`CosmeticId` y `CosmeticSlot` en el contrato; `GET /cosmetics` lo
  sirve con el logro que desbloquea cada uno):

  | Lugar | De serie | Por logro |
  |---|---|---|
  | Paño | `feltGreen` | `feltBlue` (`dailyComplete`), `feltRed` (`doubleUp`), `feltBlack` (`rankedMedal`) |
  | Dorso | `backClassic` | `backGold` (`blackjack`), `backMidnight` (`perfectSplit`) |
  | Fichas | `chipsClassic` | `chipsNeon` (`sixToFiveSurvivor`), `chipsGold` (`rankedQualified`) |
  | Título | (ninguno) | `titleRookie` (`firstWin`), `titleCoolHead` (`optimal10`), `titleByTheBook` (`optimal50`), `titleMedalist` (`rankedMedal`) |

  Todo logro desbloquea al menos un cosmético (test).
- **Lo que tiene** se deriva de sus logros cada vez; no se guarda. Así un cosmético nuevo
  ligado a un logro existente le llega a quien ya lo tenía.
- **Lo que tiene puesto** va en `user_cosmetics`; sin fila, los de serie y sin título.
  `PUT /me/cosmetics` reemplaza los cuatro a la vez: 400 `cosmetic_wrong_slot` si uno no va
  en ese lugar, 409 `cosmetic_locked` si no lo desbloqueó.
- **Títulos en los rankings:** las filas de los tres rankings (`LeaderboardRow`,
  `RankedRow`, `DailyRow`) traen `title` con el título equipado, si hay uno. Es el que lleva
  puesto ahora, no el que tenía cuando jugó.

### 8.6 Seguro (todas las mesas del servidor)

- **Cuándo:** con un As visible, después del reparto y **antes del peek**. La ronda queda en
  la fase `insurance`: no hay más acciones hasta decidir, y la oculta sigue sin verse.
- **Cuánto:** media apuesta (las apuestas son múltiplos de 10, así que siempre es entero).
  Paga 2:1 si el dealer tiene blackjack; si no, se pierde. Hacen falta fichas libres para la
  media apuesta (`allowed.insurance`); rechazarlo siempre se puede.
- **Dinero par:** con blackjack, tomar el seguro asegura +1 apuesta haya o no blackjack del
  dealer. Es el mismo seguro; el front lo puede llamar «dinero par».
- **Sin peek** (preset Europa): también se ofrece; se resuelve al final, cuando el dealer
  revela.
- **Rondas individuales:** `POST /play/rounds/{id}/insurance` `{ take, idempotencyKey }`,
  con la misma idempotencia que las acciones. `RoundView.insured` dice qué se decidió y
  `result.insuranceNet` es el neto del seguro, ya sumado a `result.net`. La ronda abandonada
  24 h lo rechaza sola.
- **Revisión (Ranked y reto diario):** la decisión del seguro no entra en `review`. El EV
  está en `analyze_hand` (`insuranceEv`) para quien quiera mostrarlo en las ayudas.
- **Salas:** ver «Especificación — Salas Casual.md», sección 5.

---

## 9. Seguridad: lista de control

- [x] La oculta y el orden del zapato jamás salen del servidor antes de tiempo (test de propiedad).
- [x] CSPRNG para barajar; la semilla del reto diario es secreta (`DAILY_SEED_SECRET`, o
      derivada de `JWT_SECRET`).
- [x] Toda apuesta y acción se valida en el servidor (límites, saldo, acciones permitidas, turno).
- [x] Transacciones con bloqueo de fila; una ronda activa por usuario; idempotencia.
- [x] Rankings sin emails; `display_name` validado y con lista de reservados.
- [x] Ranked sin ayudas durante la mano (sin `shoeCounts`; test).
- [x] OAuth: `state` + PKCE, cookie temporal cifrada, intercambio del código solo en el
      servidor, nunca vincular por email automáticamente (solo de forma explícita con
      sesión) y crear cuentas solo con email verificado.
- [x] Límite de tasa en registro, login, OAuth y acciones de juego.
- [ ] Cookies `Secure` en producción (`cookie_secure`); CORS solo para `FRONTEND_ORIGIN`.
- [x] **Sesiones revocables.** Tabla `auth_sessions (id, user_id, created_at, expires_at,
      revoked_at, last_seen_at, user_agent)`; el JWT lleva su `id` como `sid`. Cada petición
      autenticada comprueba que la fila exista, no esté revocada y no haya vencido (una
      consulta por clave primaria; `last_seen_at` se actualiza como mucho cada 5 min).
      `POST /auth/logout` revoca la sesión actual y `POST /auth/logout-all` todas las del
      usuario. Una tarea horaria borra las vencidas o revocadas hace más de un día. Los JWT
      anteriores (sin `sid`) dejaron de valer.
      Vincular Google **no** revoca las demás sesiones: si quien vincula es alguien con un
      token robado, cerraría la sesión del dueño y dejaría la suya. Cambiar la contraseña,
      cuando exista, sí debe revocar las demás.
- [x] **Vincular con una sesión robada.** Un token solo ya no basta para vincular Google:
      `POST /auth/google/link-intent` con la contraseña deja en `auth_sessions` una prueba de
      un solo uso que vence a los 5 minutos (`link_intent_until`), y `start?link=1` la gasta
      de forma atómica; sin ella vuelve con `reauth_required`. Está atada a la sesión que la
      pidió. `POST /auth/google/unlink` (con la contraseña) quita Google, se rechaza si la
      cuenta no tiene contraseña (`no_password`) y **revoca las demás sesiones**: si alguien
      había vinculado su Google con un token robado, también pierde lo que abrió con él, y
      como exige la contraseña no sirve para echar al dueño. Contraseña incorrecta → 403
      `invalid_password`, nunca 401 (para el front, 401 con sesión = sesión cerrada). Ambos
      endpoints entran en el límite de tasa de cuentas.
