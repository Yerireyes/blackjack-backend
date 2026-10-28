-- Cuentas: nombre visible, cuentas sin contraseña (Google) e identidades externas.

ALTER TABLE users
    ADD COLUMN display_name     text,
    -- Clave de unicidad: el nombre en minúsculas (Unicode), calculada por la API.
    -- No se usa lower() porque con el locale C solo pasa a minúsculas el ASCII.
    ADD COLUMN display_name_key text,
    -- false mientras el nombre sea provisional; /auth/me lo expone como needsDisplayName.
    ADD COLUMN display_name_set boolean NOT NULL DEFAULT false,
    ALTER COLUMN password_hash DROP NOT NULL;

-- Usuarios previos: nombre provisional único; tendrán que elegir uno.
UPDATE users u
SET display_name = 'Jugador' || n.rn,
    display_name_key = 'jugador' || n.rn
FROM (SELECT id, row_number() OVER (ORDER BY created_at, id) AS rn FROM users) n
WHERE u.id = n.id;

ALTER TABLE users
    ALTER COLUMN display_name SET NOT NULL,
    ALTER COLUMN display_name_key SET NOT NULL;
CREATE UNIQUE INDEX users_display_name_key ON users (display_name_key);

CREATE TABLE identities (
    id         uuid PRIMARY KEY,
    user_id    uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    provider   text NOT NULL CHECK (provider IN ('google')),
    -- "sub" del proveedor; nunca el email.
    subject    text NOT NULL,
    email      text,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (provider, subject),
    -- Una sola identidad de cada proveedor por usuario.
    UNIQUE (user_id, provider)
);
