-- Fase 3: partidas Casual y rondas jugadas en el servidor.

CREATE TABLE casual_runs (
    id          uuid PRIMARY KEY,
    user_id     uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    rules       jsonb NOT NULL,
    preset      text CHECK (preset IN ('vegas', 'europe', 'sixToFive')),
    -- Orden real de las cartas: nunca sale del servidor.
    shoe        jsonb NOT NULL,
    balance     bigint NOT NULL CHECK (balance >= 0),
    peak        bigint NOT NULL,
    hands       integer NOT NULL DEFAULT 0,
    status      text NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'retired', 'busted')),
    started_at  timestamptz NOT NULL DEFAULT now(),
    ended_at    timestamptz
);

-- Una sola partida casual activa por usuario.
CREATE UNIQUE INDEX casual_runs_one_active ON casual_runs(user_id) WHERE status = 'active';
CREATE INDEX casual_runs_started_at ON casual_runs(started_at);

CREATE TABLE rounds (
    id               uuid PRIMARY KEY,
    user_id          uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind             text NOT NULL CHECK (kind IN ('casual', 'ranked')),
    -- casual_runs.id (o la cuenta ranked).
    context_id       uuid NOT NULL,
    bet              bigint NOT NULL CHECK (bet > 0),
    -- Estado completo de la ronda, incluida la oculta: solo servidor.
    state            jsonb NOT NULL,
    status           text NOT NULL CHECK (status IN ('active', 'settled')),
    net              bigint,
    idempotency_key  uuid NOT NULL,
    -- Respuesta del reparto, para devolverla igual si se repite la clave.
    response         jsonb NOT NULL,
    created_at       timestamptz NOT NULL DEFAULT now(),
    settled_at       timestamptz,
    UNIQUE (user_id, idempotency_key)
);

-- Una sola ronda activa por usuario (sumando Casual y Ranked).
CREATE UNIQUE INDEX rounds_one_active ON rounds(user_id) WHERE status = 'active';
CREATE INDEX rounds_context ON rounds(context_id);

CREATE TABLE round_actions (
    id               uuid PRIMARY KEY,
    round_id         uuid NOT NULL REFERENCES rounds(id) ON DELETE CASCADE,
    seq              integer NOT NULL,
    action           text NOT NULL,
    idempotency_key  uuid NOT NULL,
    response         jsonb NOT NULL,
    created_at       timestamptz NOT NULL DEFAULT now(),
    UNIQUE (round_id, seq),
    UNIQUE (round_id, idempotency_key)
);
