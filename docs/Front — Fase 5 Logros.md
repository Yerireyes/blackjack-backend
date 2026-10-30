# Front — Fase 5: Logros

**Versión del 2026-10-29 (primera).** Qué cambió respecto de lo que el front conoce:

- `RoundResult` suma `unlocked?: AchievementId[]`: los logros nuevos de esa ronda.
- Endpoint nuevo `GET /me/achievements` → `MyAchievements`.
- Tipos nuevos en `src/engine/contract.ts` (generados): `AchievementId`,
  `UnlockedAchievement` y `MyAchievements`.

No cambia nada más: las rondas de Casual, Ranked y el reto diario funcionan igual. Producto:
sección 8.4 de
[Especificación — Cuentas, Casual y Ranked.md](Especificación%20—%20Cuentas,%20Casual%20y%20Ranked.md).
Este documento solo existe en `blackjack-backend/docs/`.

## Estado

| | Backend | Front |
|---|---|---|
| Logros (otorgarlos, listarlos) | Listo | Por hacer |
| Racha de decisiones óptimas | Listo | Por hacer |
| Cosméticos (recompensas de los logros) | No existe | **No hacer** |

---

## Qué añadir

### 1. Catálogo

El servidor decide cuándo se cumple cada logro; **el front pone el nombre, la descripción y
el icono**. Textos sugeridos:

| `AchievementId` | Nombre | Descripción |
|---|---|---|
| `firstWin` | Primera victoria | Gana una ronda contra el servidor. |
| `blackjack` | ¡Blackjack! | Saca un blackjack natural. |
| `perfectSplit` | Divide y vencerás | Divide y gana todas las manos de la ronda. |
| `optimal10` | Cabeza fría | 10 decisiones óptimas seguidas en Ranked o el reto diario. |
| `optimal50` | Jugador de libro | 50 decisiones óptimas seguidas en Ranked o el reto diario. |
| `sixToFiveSurvivor` | Superviviente del 6:5 | Llega a 50 manos en una partida casual que paga 6:5. |
| `doubleUp` | Doble o nada | Llega a 1.000 fichas en una partida casual. |
| `rankedQualified` | Clasificado | Juega 200 manos en una temporada ranked. |
| `rankedMedal` | Medallista | Termina una temporada ranked con medalla. |
| `dailyComplete` | Reto cumplido | Juega las 30 manos del reto diario. |

Si llega un id que el front no conoce (un logro nuevo del backend), ignorarlo o mostrarlo
genérico; no romper.

### 2. Al liquidar una ronda

- Si `result.unlocked` viene, mostrar un aviso por cada logro («¡Logro desbloqueado:
  Primera victoria!»). Falta si no hubo ninguno.
- Solo trae los **nuevos**: un logro ya obtenido no se vuelve a anunciar.
- Una respuesta repetida por idempotencia trae lo mismo que la primera; si el front ya
  mostró el aviso, no repetirlo.
- `rankedMedal` no llega en una ronda: se otorga al cerrar la temporada. Se ve en
  `GET /me/achievements`.

### 3. `GET /me/achievements` (con sesión)

```ts
interface MyAchievements {
  unlocked: { id: AchievementId; unlockedAt: string }[]; // del más antiguo al más reciente
  optimalStreak: number;      // decisiones óptimas seguidas ahora mismo
  bestOptimalStreak: number;  // la mejor racha
}
```

- Pantalla de logros: el catálogo completo, con los de `unlocked` marcados (y su fecha) y
  el resto bloqueados.
- **Racha:** cuenta solo las decisiones revisadas (Ranked y reto diario). Una decisión
  óptima suma 1; cualquier otra la deja en 0. Se puede mostrar junto a la revisión de cada
  ronda («Racha: 7»), pidiendo `GET /me/achievements` después de liquidar.

---

## Qué NO hacer

- **No calcular logros en el front** ni darlos por la revisión local: los otorga el
  servidor.
- **No construir cosméticos** todavía.
- **No editar `contract.ts` a mano.**

## Probar

- Ganar una ronda en Casual: llega `unlocked: ["firstWin"]` y en `GET /me/achievements`
  aparece. Ganar otra: ya no llega.
- Sacar un blackjack: `"blackjack"`.
- En Ranked, jugar varias manos y ver que `optimalStreak` sube con las decisiones óptimas y
  vuelve a 0 con un error.
