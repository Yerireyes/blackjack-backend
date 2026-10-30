-- Fase 5: cosméticos. Lo que el usuario tiene se deriva de sus logros (y los de serie);
-- aquí solo se guarda lo que tiene puesto. Sin fila: los de serie.

CREATE TABLE user_cosmetics (
    user_id     uuid PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    -- CosmeticId en camelCase ("feltGreen"...).
    felt        text NOT NULL,
    card_back   text NOT NULL,
    chips       text NOT NULL,
    title       text,
    updated_at  timestamptz NOT NULL DEFAULT now()
);
