# Front — Fase 1: Cuentas

**Versión del 2026-10-28 (revisión 2).** Cambios respecto de la revisión 1 (misma fecha):

- **Vincular Google ahora pide la contraseña antes:** nuevo `POST /auth/google/link-intent`,
  y `start?link=1` sin ese paso vuelve con `reauth_required`. Ver la sección 10.
- **Nuevo `POST /auth/google/unlink`** para desvincular Google (sección 10).
- Errores nuevos: 403 `invalid_password`, 409 `no_password`, 409 `already_linked` (en
  `link-intent`), 409 `not_linked`, 409 `google_disabled` y el código de vuelta
  `reauth_required` (tablas de las secciones 3 y 7).
- Sección 9 (Perfil): vincular y desvincular cambian; «Qué NO hacer» ya no prohíbe desvincular.

Revisión 1 (2026-10-28): `POST /auth/logout-all`; `POST /auth/logout` revoca la sesión en el
servidor; un 401 también puede ser una sesión cerrada desde otro dispositivo; las sesiones
anteriores a ese cambio dejaron de valer.

Qué cambió en el backend con la fase 1 y qué tiene que hacer el front, y qué **no**. El
detalle completo está en la sección 2 de
[Especificación — Cuentas, Casual y Ranked.md](Especificación%20—%20Cuentas,%20Casual%20y%20Ranked.md).
Este documento solo existe en `blackjack-backend/docs/`; el front lo enlaza y no guarda copia.

## Estado

| | Backend | Front |
|---|---|---|
| Registro, login y logout con nombre visible | Listo | Listo |
| Login con Google | Listo, **deshabilitado** hasta tener credenciales | Listo, oculto mientras esté deshabilitado |
| Vincular Google a una cuenta con contraseña | Listo | Listo |
| Cerrar sesión en todos los dispositivos | Listo | Listo |
| Pedir contraseña para vincular Google y desvincular | Listo | **Por hacer** (sección 10) |
| Casual, Ranked, rankings y récords (fases 2–5) | No existe | **No hacer todavía** |

El modo Práctica no cambia y sigue funcionando sin cuenta.

## Lo que ya está hecho en el front

`vite.config.ts` tiene un proxy: `/api` → `http://127.0.0.1:8080`. Gracias a eso, front y
API comparten el origen `http://localhost:5180` en desarrollo. La cookie de sesión y la
vuelta desde Google dependen de esto, así que el proxy no se toca.

Para arrancar todo:

```sh
# en el backend
npm run db:start
npm run api
# en el front
npm run dev
```

---

## Qué añadir

### 1. Cliente HTTP

- **URLs relativas:** `fetch('/api/v1/...')`. Nunca `http://127.0.0.1:8080/...`: sería otro
  sitio y la cookie no viajaría.
- **Sesión:** cookie httpOnly `bj_session` que pone el servidor. El front no la lee ni la
  guarda, y no hay que mandar ningún `Authorization`. Basta con
  `credentials: 'same-origin'` (es el valor por defecto de `fetch`).
- **JSON:** cuerpo JSON con `Content-Type: application/json`.
- **Errores:** siempre `{ code: string, message: string }`. `message` ya viene en español y se
  puede mostrar tal cual. Decide por `code`, no por el texto.
- **429 `rate_limited`:** el registro, el login, `PATCH /me` y todas las rutas de Google
  (incluidas `link-intent` y `unlink`) comparten un límite por IP (10 seguidas y luego una
  cada 6 s). La cabecera `Retry-After` trae los segundos de espera.
- **401 vs 403:** un 401 en una llamada con sesión siempre significa «sesión cerrada». Una
  contraseña incorrecta en una acción con sesión es **403 `invalid_password`**, y la sesión
  sigue viva.

> `src/engine/api.ts` es la API del **motor WASM**, no la HTTP. Conviene un módulo aparte
> (por ejemplo `src/account/api.ts`) para no mezclarlos.

### 2. Tipos

Estos tipos **no** están en `contract.ts`, que solo genera los del motor. Decláralos a mano:

```ts
/** Lo devuelven register, login, GET /auth/me y PATCH /me. Nunca incluye el hash. */
interface Me {
  id: string;
  email: string;
  displayName: string;
  /** El nombre es provisional (cuenta creada con Google o anterior a los nombres). */
  needsDisplayName: boolean;
  /** false en cuentas creadas con Google. */
  hasPassword: boolean;
  /** Proveedores vinculados: [] o ["google"]. */
  providers: string[];
  createdAt: string; // RFC 3339
}

interface Providers { google: boolean }

interface ApiError { code: string; message: string }
```

### 3. Endpoints (todos bajo `/api/v1`)

| Método | Ruta | Cuerpo | Éxito | Errores (`code`) |
|---|---|---|---|---|
| GET | `/auth/providers` | — | 200 `Providers` | — |
| GET | `/auth/me` | — | 200 `Me` | 401 `unauthorized` (no hay sesión: es el caso normal sin login) |
| POST | `/auth/register` | `{ email, password, displayName }` | 201 `Me` + cookie | 400 `bad_request` (email o contraseña), 400 `display_name_invalid`, 409 `email_taken`, 409 `display_name_taken`, 4xx `invalid_json` (falta un campo) |
| POST | `/auth/login` | `{ email, password }` | 200 `Me` + cookie | 401 `invalid_credentials` |
| POST | `/auth/logout` | — | 204, revoca la sesión y borra la cookie | 401 si ya no había sesión (trátalo como logout correcto) |
| POST | `/auth/logout-all` | — | 204, revoca todas las sesiones del usuario (también esta) y borra la cookie | 401 si ya no había sesión |
| PATCH | `/me` | `{ displayName }` | 200 `Me` | 400 `display_name_invalid`, 409 `display_name_taken`, 401 |
| POST | `/auth/google/link-intent` | `{ password }` | 204 (prueba de vinculación de 5 min) | 403 `invalid_password`, 409 `no_password`, 409 `already_linked`, 409 `google_disabled`, 401 |
| POST | `/auth/google/unlink` | `{ password }` | 200 `Me` (sin `"google"`) | 403 `invalid_password`, 409 `no_password`, 409 `not_linked`, 401 |

`/auth/login` responde lo mismo si el email no existe, si la contraseña está mal o si la
cuenta es de Google y no tiene contraseña. No distingas esos casos en la interfaz.

### 4. Estado de sesión (store de zustand)

- **Al arrancar:** pedir `GET /auth/providers` y `GET /auth/me` en paralelo. Si `/auth/me`
  responde 200 hay sesión; si responde 401, el usuario es anónimo.
- **Después de register, login o `PATCH /me`:** guardar el `Me` de la respuesta. No hace
  falta volver a pedir `/auth/me`.
- **Después de logout:** limpiar el usuario.
- **Si cualquier llamada con sesión responde 401:** la sesión venció (dura 7 días) o se
  cerró desde otro dispositivo. Pasar a anónimo.

### 5. Registro y login

- **Campos del registro:** email, contraseña (8–128 caracteres) y nombre visible.
- **Validación en el cliente (opcional, solo para avisar antes):** el servidor es quien
  decide. Reglas del nombre:
  - 3–20 caracteres.
  - Letras latinas (incluidas tildes, ñ, ü), dígitos, `_`, `-` y espacios simples entre
    palabras.
  - Al menos una letra o dígito.
  - No duplicar las reglas de nombres reservados: llegan como `display_name_invalid`.
- **Errores:** `display_name_taken` y `display_name_invalid` van junto al campo del nombre;
  `email_taken` junto al email.

### 6. Botón de Google

- **Mostrarlo solo si `providers.google === true`.** Hoy es `false`: no hay credenciales y el
  botón no debe verse.
- **Al pulsarlo, navegar con `window.location.href = '/api/v1/auth/google/start'`.** Tiene que
  ser una navegación completa, **no** un `fetch`: el servidor redirige a Google, y Google
  vuelve a la app.

### 7. Vuelta de Google

Google y la API devuelven al usuario a `http://localhost:5180/` con uno de estos parámetros:

| Query | Qué significa | Qué hacer |
|---|---|---|
| `?login=ok` | Sesión iniciada | Pedir `GET /auth/me` |
| `?link=ok` | Google quedó vinculado a la cuenta actual | Pedir `GET /auth/me` y avisar «Google vinculado» |
| `?login=error&code=…` / `?link=error&code=…` | Falló | Mostrar el mensaje de la tabla siguiente |

Después de leerlos, quítalos de la URL con `history.replaceState`, para que recargar la página
no repita el aviso.

| `code` | Mensaje sugerido |
|---|---|
| `account_exists` | «Ya hay una cuenta con ese email. Entra con tu contraseña y vincula Google desde tu perfil.» |
| `email_unverified` | «Tu cuenta de Google no tiene el email verificado.» |
| `cancelled` | Nada, o «Inicio de sesión cancelado». |
| `invalid_state` | «El inicio de sesión caducó. Inténtalo de nuevo.» |
| `identity_in_use` | «Esa cuenta de Google ya está vinculada a otro usuario.» |
| `already_linked` | «Tu cuenta ya tiene otra cuenta de Google vinculada.» |
| `unauthorized` | «Inicia sesión para vincular Google.» |
| `reauth_required` | «Confirma tu contraseña para vincular Google.» Volver a pedirla (sección 10). |
| `google_disabled` | «El acceso con Google no está disponible.» |
| `google_error`, `internal` | «No se pudo completar el acceso con Google. Inténtalo de nuevo.» |

### 8. Nombre provisional

Si `me.needsDisplayName` es `true`, la cuenta tiene un nombre automático (`Jugador4821`).
Pasa con las cuentas creadas con Google y con las que ya existían antes de esta fase.

- Mostrar un diálogo para elegir nombre, con `PATCH /me`. Al responder, `needsDisplayName`
  pasa a `false`.
- Por ahora se puede posponer. Cuando existan los rankings, será obligatorio antes de
  jugar Casual o Ranked.

### 9. Perfil

- Mostrar `displayName` y permitir cambiarlo con `PATCH /me`. Cambiar solo mayúsculas del
  propio nombre está permitido.
- **Vincular Google** solo si `providers.google` es `true`, `me.providers` no incluye
  `"google"` y `me.hasPassword` es `true`. Pide la contraseña primero (sección 10).
- Si `me.providers` incluye `"google"`: mostrar «Google vinculado» y, si `me.hasPassword` es
  `true`, el botón «Desvincular» (sección 10). Si no tiene contraseña, sin botón: Google es
  su única forma de entrar.
- **«Cerrar sesión en todos los dispositivos»** con `POST /auth/logout-all`. Conviene pedir
  confirmación. Al responder 204, limpiar el usuario igual que en logout.

### 10. Vincular y desvincular Google con contraseña

**Vincular** (dos pasos):

1. Pedir la contraseña en un diálogo y llamar `POST /auth/google/link-intent` con
   `{ password }` (un `fetch` normal).
   - **204:** el servidor guardó en **esta sesión** una prueba de vinculación. No devuelve
     nada que el front tenga que guardar o pasar: la prueba vive en el servidor, atada a la
     cookie de sesión.
   - **403 `invalid_password`:** mostrar el error en el diálogo. La sesión sigue viva.
   - **409 `already_linked`:** la cuenta ya tiene Google; refrescar `/auth/me`.
   - **409 `no_password`:** la cuenta no tiene contraseña (no debería pasar si el botón se
     oculta según `hasPassword`).
   - **409 `google_disabled`:** Google se deshabilitó; ocultar el botón.
2. Justo después, navegar con `window.location.href = '/api/v1/auth/google/start?link=1'`.
   Vuelve con `?link=ok` o `?link=error&code=…` (sección 7).

Sobre la prueba de vinculación:

- **Dura 5 minutos** desde el 204 y es de **un solo uso**: la gasta la navegación a
  `start?link=1`, salga como salga después.
- **Es de la sesión que la pidió.** Otra pestaña con la misma cookie la comparte; otro
  dispositivo del mismo usuario, no.
- **Si el usuario cancela en Google** (vuelve con `?link=error&code=cancelled`) o la vuelta
  falla por cualquier motivo, la prueba ya se gastó: para reintentar hay que volver a pedir
  la contraseña.
- **Sin prueba vigente** (no se pidió, venció o ya se gastó), `start?link=1` no va a Google y
  vuelve enseguida con `?link=error&code=reauth_required`. Lo normal es reabrir el diálogo de
  contraseña.

**Desvincular:** pedir la contraseña con una confirmación («Ya no podrás entrar con Google»)
y llamar `POST /auth/google/unlink` con `{ password }`.

- **200 `Me`:** guardar el `Me` de la respuesta (ya sin `"google"` en `providers`).
- **Cierra las demás sesiones del usuario**, no la actual. Si alguien había vinculado su
  propio Google con un token robado, pierde las sesiones que abrió con él. Convendría avisar:
  «Se cerró la sesión en tus otros dispositivos».
- **403 `invalid_password`:** error en el diálogo; la sesión sigue viva.
- **409 `no_password`:** la cuenta no tiene contraseña; quitar Google la dejaría sin acceso.
  No debería pasar si el botón se oculta según `hasPassword`.
- **409 `not_linked`:** ya no estaba vinculado; refrescar `/auth/me`.

---

## Qué NO hacer

- **No guardar ni leer el token.** Nada de `localStorage`, `sessionStorage`, cabeceras
  `Authorization` ni lectura de `document.cookie`: la sesión es una cookie httpOnly.
- **No integrar el SDK de Google** (Google Identity Services, One Tap ni `gapi`), ni llamar a
  Google desde el navegador. Todo el intercambio OAuth lo hace el servidor.
- **No usar `fetch` contra `/auth/google/start` ni `/auth/google/callback`.** Son navegaciones.
- **No apuntar a `127.0.0.1:8080`** ni cambiar el proxy de Vite.
- **No vincular Google de forma automática** ni ofrecer «continuar» cuando llega
  `account_exists`. El backend lo bloquea a propósito: el email de una cuenta con contraseña
  nunca se verificó. El camino es entrar con contraseña y vincular desde el perfil.
- **No agregar pantallas sin endpoint:** recuperar o cambiar contraseña, crear contraseña en
  una cuenta de Google, borrar la cuenta y Facebook. Ninguna existe en el backend todavía.
- **No guardar la contraseña** del diálogo de `link-intent` o `unlink` más allá de la llamada.
- **No construir todavía Casual, Ranked, rankings, récords ni temporadas.** Los endpoints
  `/casual/*`, `/ranked/*`, `/play/*` y `/me/records` de la especificación son de las
  fases 2–4 y no existen.
- **No exigir cuenta para Práctica.** Sigue igual y sin login.
- **No mostrar el email en nada público** (más adelante, rankings). Lo público es `displayName`.
- **No editar `contract.ts` a mano** ni meter ahí los tipos de cuenta.

## Probar sin credenciales de Google

- Con Google deshabilitado, `/auth/providers` devuelve `{ "google": false }` y el botón no se
  ve. Es lo que hay que comprobar hoy.
- Los avisos de vuelta se prueban abriendo la URL a mano, por ejemplo
  `http://localhost:5180/?login=error&code=account_exists` o
  `http://localhost:5180/?link=ok`.
- Para forzar el diálogo de nombre provisional, `PATCH /me` no sirve: siempre deja
  `needsDisplayName` en `false`. Se puede simular en el store, o probar con un usuario creado
  antes de esta fase.
