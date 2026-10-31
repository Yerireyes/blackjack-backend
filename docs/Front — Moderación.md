# Front — Moderación

**Versión del 2026-10-31 (primera).** Qué cambió respecto de lo que el front conoce:

- `Me` suma `isAdmin: boolean` (el tipo `Me` lo declara el front a mano: agregar el campo).
- Cualquier usuario puede **reportar** a otro: `POST /reports`.
- Panel de **administración** (solo con `isAdmin`): reportes, búsqueda de usuarios, cambio
  de nombre forzado y suspensión de cuentas.
- Error nuevo al iniciar sesión: 403 `account_banned` (y `?login=error&code=account_banned`
  al volver de Google).

Producto: sección 8.9 de
[Especificación — Cuentas, Casual y Ranked.md](Especificación%20—%20Cuentas,%20Casual%20y%20Ranked.md).
Estos tipos no salen de `contract.ts` (son de cuentas, como `Me`): se declaran a mano con
las formas de abajo. Este documento solo existe en `blackjack-backend/docs/`.

## Qué añadir

### 1. Reportar (cualquier usuario con sesión)

`POST /reports` `{ displayName, reason, details? }` → 202.

- `reason`: `"name"` (nombre ofensivo), `"behavior"`, `"cheating"` u `"other"`.
- `details`: opcional, hasta 300 caracteres.
- Errores: 404 `user_not_found`, 400 (motivo inválido, detalle largo o uno mismo), 409
  `report_exists` («Ya lo reportaste; un administrador lo va a revisar»), 429.
- Dónde ponerlo: un menú «Reportar» junto al nombre en los rankings, en la lista de amigos y
  en los jugadores de una sala.

### 2. Cuenta suspendida

- `POST /auth/login` → 403 `account_banned`: «Tu cuenta está suspendida.»
- Google: vuelve con `?login=error&code=account_banned`.
- Si la suspenden con la sesión abierta, la siguiente llamada da 401 (como una sesión
  cerrada).

### 3. Panel de administración (solo si `me.isAdmin`)

Todas responden 403 `not_admin` a quien no es administrador.

| Método | Ruta | Cuerpo | Éxito |
|---|---|---|---|
| GET | `/admin/reports?status=open` (o `resolved`) | — | 200 `Report[]` (los últimos 200) |
| POST | `/admin/reports/{id}/dismiss` | — | 204 (404 si ya estaba resuelto) |
| GET | `/admin/users?name=<comienzo>` | — | 200 `AdminUser[]` (hasta 20) |
| POST | `/admin/users/{id}/rename` | — | 200 `{ displayName }` (el provisional) |
| POST | `/admin/users/{id}/ban` | `{ reason }` (1–300) | 204; 409 `cannot_ban_admin` |
| POST | `/admin/users/{id}/unban` | — | 204 |

```ts
interface Report {
  id: string;
  reporter: { id: string; displayName: string };
  target: { id: string; displayName: string };
  reason: "name" | "behavior" | "cheating" | "other";
  details?: string;
  createdAt: string;
  resolvedAt?: string;
  action?: "dismiss" | "rename" | "ban";
}
interface AdminUser {
  id: string; displayName: string; email: string; createdAt: string;
  bannedAt?: string; banReason?: string;
  openReports: number;
}
```

- Pantalla sugerida: pestaña «Reportes» (lista con Descartar / Cambiar nombre / Suspender
  sobre el reportado) y pestaña «Usuarios» (buscador por nombre con las mismas acciones y
  «Levantar suspensión»).
- Cambiar el nombre o suspender cierra todos los reportes abiertos sobre esa persona.
- Pedir confirmación antes de suspender, con el motivo obligatorio.

---

## Qué NO hacer

- **No mostrar el panel** si `isAdmin` es `false` (igual el backend lo rechaza).
- **No mostrar emails** fuera del panel de administración.

## Probar

- Con `ADMIN_EMAILS` apuntando a una cuenta de prueba: reportar a otra cuenta desde una
  tercera, verlo en el panel, cambiarle el nombre (al entrar le pide uno nuevo), suspenderla
  (no puede entrar y desaparece de los rankings) y levantar la suspensión.
