# Front — Fase 5: Cosméticos

**Versión del 2026-10-29 (primera).** Qué cambió respecto de lo que el front conoce:

- Cosméticos desbloqueados por logros: **paño**, **dorso de las cartas**, **fichas** y
  **título**. Solo cambian cómo se ve la mesa: nunca dan fichas ni ventaja.
- Endpoints nuevos: `GET /cosmetics` (catálogo, público), `GET /me/cosmetics` y
  `PUT /me/cosmetics`.
- Tipos nuevos en `src/engine/contract.ts` (generados): `CosmeticId`, `CosmeticSlot`,
  `CosmeticItem`, `EquippedCosmetics` y `MyCosmetics`.
- Errores nuevos: `cosmetic_locked` y `cosmetic_wrong_slot`.

Se apoya en «Front — Fase 5 Logros.md». Producto: sección 8.5 de
[Especificación — Cuentas, Casual y Ranked.md](Especificación%20—%20Cuentas,%20Casual%20y%20Ranked.md).
Este documento solo existe en `blackjack-backend/docs/`.

## Estado

| | Backend | Front |
|---|---|---|
| Catálogo y desbloqueo por logros | Listo | Por hacer |
| Elegir y guardar lo que se tiene puesto | Listo | Por hacer |
| Título junto al nombre en los rankings | No existe | **No hacer** |

---

## Qué añadir

### 1. Catálogo (`GET /cosmetics`, sin sesión)

Devuelve `CosmeticItem[]` en orden: `{ id, slot, unlockedBy? }`. `unlockedBy` es el
`AchievementId` que lo desbloquea; falta en los que vienen de serie.

| `slot` | De serie | Por logro (`unlockedBy`) | Sugerencia de diseño |
|---|---|---|---|
| `felt` | `feltGreen` | `feltBlue` (`dailyComplete`), `feltRed` (`doubleUp`), `feltBlack` (`rankedMedal`) | Paño verde clásico; azul; rojo; negro |
| `cardBack` | `backClassic` | `backGold` (`blackjack`), `backMidnight` (`perfectSplit`) | Dorso clásico; dorado; azul medianoche |
| `chips` | `chipsClassic` | `chipsNeon` (`sixToFiveSurvivor`), `chipsGold` (`rankedQualified`) | Fichas clásicas; neón; doradas |
| `title` | (ninguno) | `titleRookie` (`firstWin`), `titleCoolHead` (`optimal10`), `titleByTheBook` (`optimal50`), `titleMedalist` (`rankedMedal`) | «Novato», «Cabeza fría», «De libro», «Medallista» |

Los nombres, colores e imágenes los pone el front. Si llega un id que no conoce, ignorarlo.

### 2. `GET /me/cosmetics` (con sesión)

```ts
interface MyCosmetics {
  owned: CosmeticId[];          // los que tiene (de serie + por logros)
  equipped: {
    felt: CosmeticId;
    cardBack: CosmeticId;
    chips: CosmeticId;
    title: CosmeticId | null;   // sin título: null
  };
}
```

- Pedirlo al iniciar sesión y aplicar `equipped` a la mesa en **todos los modos**, incluida
  Práctica.
- Después de un `result.unlocked` con logros nuevos, volver a pedirlo: puede haber
  cosméticos nuevos en `owned`.
- Sin sesión, usar los de serie (`feltGreen`, `backClassic`, `chipsClassic`, sin título).

### 3. `PUT /me/cosmetics` (con sesión)

- Cuerpo: el objeto `equipped` **completo** (los cuatro campos; `title: null` para quitarlo).
  Responde `MyCosmetics` con lo guardado.
- 400 `cosmetic_wrong_slot`: un id en el lugar equivocado (p. ej. un paño en `cardBack`).
- 409 `cosmetic_locked`: un id que todavía no tiene.

### 4. Pantalla sugerida

En el perfil o en «Logros»: una sección por lugar con todos los del catálogo. Los que
tiene se pueden elegir; los bloqueados se ven atenuados con el logro que los desbloquea
(«Saca un blackjack para desbloquearlo»).

### 5. Mensajes sugeridos

| `code` | Mensaje |
|---|---|
| `cosmetic_locked` | «Todavía no lo desbloqueaste.» (y recargar `GET /me/cosmetics`) |
| `cosmetic_wrong_slot` | Error del front: no debería pasar. |

---

## Qué NO hacer

- **No guardar la elección solo en `localStorage`** cuando hay sesión: se guarda en el
  servidor para que se vea igual en otro dispositivo. Sin sesión sí se puede recordar en el
  navegador.
- **No dar ventaja con un cosmético** (ni mostrar ayudas en Ranked por llevar uno).
- **No mostrar el título en los rankings** todavía: el backend no lo manda en las filas.
- **No editar `contract.ts` a mano.**

## Probar

- Cuenta nueva: `owned` trae solo los tres de serie.
- Ganar una ronda (logro `firstWin`): `GET /me/cosmetics` suma `titleRookie`; ponérselo con
  `PUT` y recargar la página: sigue puesto.
- Intentar poner un cosmético bloqueado: `cosmetic_locked`.
