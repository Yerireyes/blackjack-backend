-- Esquema inicial: usuarios, sesiones de juego y manos registradas.

CREATE TABLE users (
    id            uuid PRIMARY KEY,
    email         text NOT NULL,
    password_hash text NOT NULL,
    created_at    timestamptz NOT NULL DEFAULT now()
);
-- Email único sin distinguir mayúsculas.
CREATE UNIQUE INDEX users_email_key ON users (lower(email));

CREATE TABLE game_sessions (
    id         uuid PRIMARY KEY,
    user_id    uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    rules      jsonb NOT NULL,
    mode       text NOT NULL CHECK (mode IN ('player', 'dealer')),
    started_at timestamptz NOT NULL DEFAULT now(),
    ended_at   timestamptz
);
CREATE INDEX game_sessions_user_id_idx ON game_sessions (user_id, started_at DESC);

CREATE TABLE hands (
    id            uuid PRIMARY KEY,
    session_id    uuid NOT NULL REFERENCES game_sessions (id) ON DELETE CASCADE,
    player_cards  smallint[] NOT NULL,
    dealer_up     smallint NOT NULL CHECK (dealer_up BETWEEN 0 AND 9),
    dealer_final  smallint[] NOT NULL,
    chosen_action text NOT NULL CHECK (chosen_action IN ('stand', 'hit', 'double', 'split', 'surrender')),
    best_action   text NOT NULL CHECK (best_action IN ('stand', 'hit', 'double', 'split', 'surrender')),
    -- Tal como los devolvió el motor, sin redondear.
    ev_chosen     double precision NOT NULL,
    ev_best       double precision NOT NULL,
    exact         boolean NOT NULL,
    result        text NOT NULL CHECK (result IN ('win', 'push', 'lose')),
    created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX hands_session_id_idx ON hands (session_id, created_at);
