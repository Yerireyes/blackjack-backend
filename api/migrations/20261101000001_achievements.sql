-- Fase 5: logros. Se otorgan en el servidor al liquidar rondas (y al cerrar temporadas).

CREATE TABLE user_achievements (
    user_id      uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    -- AchievementId en camelCase ("firstWin", "optimal10"...).
    achievement  text NOT NULL,
    unlocked_at  timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, achievement)
);

-- Contadores que necesitan los logros entre rondas.
CREATE TABLE player_stats (
    user_id              uuid PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    -- Decisiones óptimas seguidas en Ranked y el reto diario (las revisadas).
    optimal_streak       integer NOT NULL DEFAULT 0,
    best_optimal_streak  integer NOT NULL DEFAULT 0
);
