-- Fase 5: reto diario. Mismas reglas y mismo zapato para todos ese día (UTC); un intento
-- de 30 manos por usuario y día.

CREATE TABLE daily_attempts (
    -- rounds.context_id de las rondas del reto.
    id           uuid PRIMARY KEY,
    user_id      uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    day          date NOT NULL,
    -- Orden real de las cartas: nunca sale del servidor.
    shoe         jsonb NOT NULL,
    -- Barajadas hechas: la siguiente usa la semilla del día con este número.
    shuffles     integer NOT NULL DEFAULT 0,
    balance      bigint NOT NULL CHECK (balance >= 0),
    peak         bigint NOT NULL,
    hands        integer NOT NULL DEFAULT 0,
    status       text NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'finished')),
    started_at   timestamptz NOT NULL DEFAULT now(),
    finished_at  timestamptz,
    UNIQUE (user_id, day)
);

CREATE INDEX daily_attempts_ranking ON daily_attempts(day, balance DESC)
    WHERE status = 'finished';

ALTER TABLE rounds DROP CONSTRAINT rounds_kind_check;
ALTER TABLE rounds ADD CONSTRAINT rounds_kind_check
    CHECK (kind IN ('casual', 'ranked', 'daily'));
