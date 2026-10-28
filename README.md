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

## Instalación en una computadora nueva (Windows)

Clona este repo y el del front uno al lado del otro: los scripts esperan el front en
`../Frontend` (contrato TS y dependencia `file:../Backend/engine-wasm/pkg`).

```text
Blackjack/
├── Backend/    ← este repo
└── Frontend/
```

Después de instalar cada herramienta, **abre una terminal nueva** para que tome el `PATH`.
No hace falta `npm install`: los scripts de Node no tienen dependencias.

### 1. Node y Visual C++ Redistributable

- **Node 18+**: `winget install OpenJS.NodeJS.LTS`
- **Visual C++ Redistributable x64** (lo necesita `wasm-pack`):
  `winget install Microsoft.VCRedist.2015+.x64`

### 2. Rust

Hay dos opciones. La **A** es la que se usó en la máquina original y no pide permisos de
administrador.

**A. Toolchain GNU + MinGW (sin admin)**

1. Descarga `rustup-init.exe` de <https://rustup.rs> y ejecútalo así:

   ```powershell
   .\rustup-init.exe -y --default-host x86_64-pc-windows-gnu --profile minimal -c clippy -c rustfmt -t wasm32-unknown-unknown
   ```

2. Descarga MinGW-w64 de WinLibs (<https://github.com/brechtsanders/winlibs_mingw/releases>,
   archivo `winlibs-x86_64-posix-seh-gcc-*-mingw-w64ucrt-*.zip`), descomprímelo en
   `%LOCALAPPDATA%\Programs\winlibs` y agrega al `PATH` de usuario
   `%LOCALAPPDATA%\Programs\winlibs\mingw64\bin`. Sin esto algunas dependencias fallan
   con `error calling dlltool 'dlltool.exe': program not found`.

**B. Toolchain MSVC (pide admin)**

1. Instala Visual Studio Build Tools con la carga de trabajo "Desarrollo para el
   escritorio con C++".
2. Instala Rust desde <https://rustup.rs> con las opciones por defecto.
3. Ejecuta `rustup target add wasm32-unknown-unknown`.

Con esta opción no hace falta MinGW.

### 3. wasm-pack y sqlx-cli

- **wasm-pack**: descarga `wasm-pack-v*-x86_64-pc-windows-msvc.tar.gz` de
  <https://github.com/rustwasm/wasm-pack/releases> y copia `wasm-pack.exe` a
  `%USERPROFILE%\.cargo\bin`. El paquete de npm no funcionó en Windows.
- **sqlx-cli**:

  ```sh
  cargo install sqlx-cli --version 0.9.0 --locked --no-default-features --features postgres,rustls
  ```

Para comprobar todo: `rustc --version`, `wasm-pack --version`, `sqlx --version`,
`node --version`.

### 4. PostgreSQL 18

**Opción portable (sin admin; es la que usan `npm run db:*`)**

1. Descarga el zip de binarios de Windows x64 desde
   <https://www.enterprisedb.com/download-postgresql-binaries>.
2. Descomprímelo de forma que exista
   `%LOCALAPPDATA%\Programs\PostgreSQL18\pgsql\bin\postgres.exe`.
3. Crea el clúster y arráncalo. `initdb` pide la contraseña del superusuario `postgres`:

   ```powershell
   $pg = "$env:LOCALAPPDATA\Programs\PostgreSQL18"
   & "$pg\pgsql\bin\initdb.exe" -D "$pg\data" -U postgres -W -E UTF8 --locale=C -A scram-sha-256
   npm run db:start
   ```

No queda como servicio de Windows: después de cada reinicio hay que ejecutar
`npm run db:start`.

**Opción con instalador** (<https://www.postgresql.org/download/windows/>): queda como
servicio y arranca solo. En ese caso no uses `npm run db:*`.

### 5. Base de datos y `.env`

Crea el usuario y la base. Usa una contraseña solo con letras y números, porque va
dentro de una URL. `CREATEDB` lo necesitan los tests de la API, que crean una base
temporal por test.

```powershell
& "$env:LOCALAPPDATA\Programs\PostgreSQL18\pgsql\bin\psql.exe" -h localhost -U postgres `
  -c "CREATE ROLE blackjack LOGIN CREATEDB PASSWORD 'TU_CONTRASENA';" `
  -c "CREATE DATABASE blackjack OWNER blackjack;"
```

Si instalaste PostgreSQL con el instalador, `psql.exe` está en
`C:\Program Files\PostgreSQL\18\bin`.

Copia [.env.example](.env.example) a `.env` (que no se versiona) y completa estos dos
valores:

- `DATABASE_URL=postgres://blackjack:TU_CONTRASENA@localhost:5432/blackjack`
- `JWT_SECRET=`: una clave aleatoria de 32 caracteres o más. Puedes generarla con:

  ```powershell
  $b = New-Object byte[] 48; [Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($b); [Convert]::ToBase64String($b)
  ```

#### Login con Google (opcional, gratis)

Sin `GOOGLE_CLIENT_ID` y `GOOGLE_CLIENT_SECRET`, Google queda deshabilitado:
`/auth/providers` devuelve `{ "google": false }` y el front oculta el botón. Para
activarlo no hace falta una cuenta de pago ni activar la facturación:

1. En <https://console.cloud.google.com>, crea un proyecto.
2. En **APIs y servicios → Pantalla de consentimiento de OAuth**, elige *Externo*, completa
   el nombre de la app y tu email, y agrega los permisos `openid`, `email` y `profile`. En
   modo *Testing* solo pueden entrar los usuarios de prueba que agregues (hasta 100).
   Publicar la app no requiere la verificación de Google con estos permisos.
3. En **Credenciales → Crear credenciales → ID de cliente de OAuth**, elige *Aplicación web*.
   En *URI de redirección autorizados* pon `http://localhost:5180/api/v1/auth/google/callback`.
   Pasa por el proxy de Vite del front, que reenvía `/api` a la API. Para producción agrega
   también `https://<dominio>/api/v1/auth/google/callback`.
4. Copia el ID y el secreto a `GOOGLE_CLIENT_ID` y `GOOGLE_CLIENT_SECRET` en `.env`, y
   reinicia la API.

### 6. Primer arranque

```sh
npm run db:migrate       # crea las tablas (antes de compilar la API: sqlx verifica las consultas contra la base)
npm test                 # todo debe pasar
npm run build:wasm       # genera engine-wasm/pkg para el front
npm run gen:contract     # contract.ts (y lo copia a ../Frontend si ya existe)
npm run api              # API en http://127.0.0.1:8080/api/v1
```

Para compilar la API sin la base arrancada (por ejemplo en CI), usa
`SQLX_OFFLINE=true`. Así se usan los metadatos de `.sqlx/`.

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
| POST | `/auth/register` | Crea usuario (argon2id, `displayName`) y deja la sesión iniciada | No |
| POST | `/auth/login` | Crea una sesión revocable: JWT (HS256, 7 días) en cookie `bj_session` httpOnly | No |
| POST | `/auth/logout` | Revoca la sesión actual y borra la cookie | Sí |
| POST | `/auth/logout-all` | Revoca todas las sesiones del usuario (todos los dispositivos) | Sí |
| GET | `/auth/me` | Usuario de la sesión: nombre, `needsDisplayName`, `providers`, `hasPassword` | Sí |
| PATCH | `/me` | Cambia el nombre visible | Sí |
| GET | `/auth/providers` | `{ google }`: qué logins externos están habilitados | No |
| GET | `/auth/google/start[?link=1]` · `/auth/google/callback` | Login o vinculación con Google (redirige al front) | No / Sí con `link=1` |
| POST | `/auth/google/link-intent` | Confirma la contraseña antes de vincular (prueba de un solo uso, 5 min) | Sí |
| POST | `/auth/google/unlink` | Quita Google con la contraseña; revoca las demás sesiones | Sí |
| POST · GET | `/sessions` | Crea una sesión con sus reglas · lista las del usuario | Sí |
| POST | `/sessions/{id}/hands` | Registra una mano (cartas, acción elegida y óptima, EVs) | Sí |
| GET | `/sessions/{id}/stats` | Manos, % óptimas, EV perdido, decisiones aproximadas, W/P/L | Sí |
| POST | `/sessions/{id}/end` | Cierra la sesión (ya no admite manos) | Sí |
| GET | `/health` | Estado del servicio y de la base | No |

- Errores siempre como `{ "code": "...", "message": "..." }`; `EngineError` → 422.
- `/analyze/*` corre en `spawn_blocking` con límite de 5 s y límite de tasa por IP
  (`ANALYZE_REPLENISH_MS`, `ANALYZE_BURST`). Las rutas de cuenta (registro, login,
  Google y `PATCH /me`) tienen su propio límite (`AUTH_BURST`). La IP sale de
  `X-Forwarded-For` solo si la conexión viene de `TRUSTED_PROXIES` (ver producción).- Detalle de cuentas, nombres y Google (incluido por qué nunca se vincula por email
  automáticamente) en la sección 2 de
  [docs/Especificación — Cuentas, Casual y Ranked.md](docs/).
- **Front y API comparten origen.** El front llama con URLs relativas (`/api/v1/...`) y
  `credentials: "same-origin"` (el valor por defecto de `fetch`). En desarrollo lo resuelve
  el proxy de Vite (`npm run dev` y `npm run preview`). En producción, el reverse proxy
  sirve el front y reenvía `/api` (ver [Despliegue en producción](#despliegue-en-producción)).
  La cookie de sesión y la vuelta del login con Google dependen de esto.
- CORS queda restringido a `FRONTEND_ORIGIN` solo como resguardo; con el mismo origen el
  navegador no lo usa.
- Una sesión ajena responde 404 (no revela que existe).
- Cuerpo máximo 64 KiB; timeout global de 10 s.
- Tests de punta a punta en `api/tests/http.rs` con `#[sqlx::test]` (una base nueva por
  test; el rol `blackjack` tiene `CREATEDB` para eso).

## Despliegue en producción

Front y API van bajo el mismo dominio: un reverse proxy sirve el `dist/` del front en `/` y
reenvía `/api` a la API, que escucha solo en local (`BIND_ADDR=127.0.0.1:8080`).

`.env` del servidor (además de `DATABASE_URL` y `JWT_SECRET`):

```sh
COOKIE_SECURE=true
FRONTEND_ORIGIN=https://<dominio>
# Solo si Google está habilitado; la misma URI debe estar autorizada en Google Cloud.
GOOGLE_REDIRECT_URI=https://<dominio>/api/v1/auth/google/callback
```

`FRONTEND_ORIGIN` es adonde vuelve el usuario después del login con Google
(`https://<dominio>/?login=ok`). Sin barra final.

**Caddy** (obtiene y renueva el certificado HTTPS solo):

```caddyfile
<dominio> {
    handle /api/* {
        reverse_proxy 127.0.0.1:8080
    }
    handle {
        root * /srv/blackjack/dist
        try_files {path} /index.html
        file_server
    }
}
```

**Nginx** (equivalente, con el certificado ya configurado):

```nginx
server {
    listen 443 ssl;
    server_name <dominio>;
    # ssl_certificate / ssl_certificate_key ...

    location /api/ {
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }

    location / {
        root /srv/blackjack/dist;
        try_files $uri /index.html;
    }
}
```

`/api/...` no lleva barra final en `proxy_pass`, así la ruta llega completa a la API. El
`.wasm` del motor debe servirse con `Content-Type: application/wasm` (Caddy y Nginx ya lo
hacen por defecto).

**IP del cliente (límite de tasa).** Detrás del proxy, la conexión siempre viene de
`127.0.0.1`. Por eso la API lee `X-Forwarded-For`, pero solo si la conexión viene de un proxy
de confianza (`TRUSTED_PROXIES`: IPs separadas por comas; por defecto `127.0.0.1,::1`;
vacía, no confía en nadie). Recorre la cabecera de derecha a izquierda y toma la primera IP
que no sea un proxy de confianza: Caddy y Nginx agregan la IP real al final, y lo que esté a
la izquierda lo pudo escribir el cliente. Si el proxy está en otra máquina (o en otro
contenedor), agrega su IP a `TRUSTED_PROXIES`. La API nunca debe quedar expuesta
directamente con el proxy en la misma IP que el cliente podría usar.

## Rendimiento

Medido con `cargo bench` en la máquina de desarrollo: `analyze_hand` sin dividir
~55–70 ms con 1–8 mazos (objetivo < 100 ms); con dividir, ≤ ~300 ms en el peor caso
gracias al presupuesto del exacto (objetivo < 500 ms). En WASM, 10,6 vs 10 con 6
mazos tarda ~1–2 ms.

## Pendiente

- Seguro (`insuranceEv`) y caché LRU entre llamadas en el worker.
- CI: `cargo test`, `npm run check:contract`, `SQLX_OFFLINE=true` y comparación de
  benchmarks (> 20 %).
