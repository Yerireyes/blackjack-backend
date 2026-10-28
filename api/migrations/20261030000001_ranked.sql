-- Fase 4: Ranked por temporadas mensuales.

CREATE TABLE seasons (
    -- "AAAA-MM".
    id         text PRIMARY KEY CHECK (id ~ '^\d{4}-\d{2}$'),
    starts_at  timestamptz NOT NULL,
    ends_at    timestamptz NOT NULL,
    closed_at  timestamptz
);

CREATE TABLE ranked_accounts (
    -- rounds.context_id de las rondas ranked.
    id              uuid PRIMARY KEY,
    user_id         uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    season_id       text NOT NULL REFERENCES seasons(id),
    -- Orden real de las cartas: nunca sale del servidor.
    shoe            jsonb NOT NULL,
    balance         bigint NOT NULL CHECK (balance >= 0),
    peak            bigint NOT NULL,
    hands           integer NOT NULL DEFAULT 0,
    refilled_total  bigint NOT NULL DEFAULT 0,
    last_refill_at  timestamptz,
    -- Lo que ordena el ranking: recargar permite seguir jugando, pero no suma.
    net             bigint GENERATED ALWAYS AS (balance - 500 - refilled_total) STORED,
    created_at      timestamptz NOT NULL DEFAULT now(),
    UNIQUE (user_id, season_id)
);

CREATE INDEX ranked_accounts_ranking ON ranked_accounts(season_id, net DESC);

CREATE TABLE ranked_refills (
    id          uuid PRIMARY KEY,
    account_id  uuid NOT NULL REFERENCES ranked_accounts(id) ON DELETE CASCADE,
    amount      bigint NOT NULL CHECK (amount > 0),
    at          timestamptz NOT NULL DEFAULT now()
);

-- Posiciones finales al cerrar la temporada, solo de quienes tenían el mínimo de manos.
CREATE TABLE season_results (
    season_id  text NOT NULL REFERENCES seasons(id),
    user_id    uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    rank       integer NOT NULL,
    net        bigint NOT NULL,
    hands      integer NOT NULL,
    medal      text CHECK (medal IN ('gold', 'silver', 'bronze', 'top10')),
    PRIMARY KEY (season_id, user_id)
);

CREATE INDEX season_results_user ON season_results(user_id);
