-- Amigos: solicitudes, amistades (una fila por cada lado) e invitaciones a salas.

CREATE TABLE friend_requests (
    from_id     uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    to_id       uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at  timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (from_id, to_id),
    CHECK (from_id <> to_id)
);
CREATE INDEX friend_requests_to ON friend_requests(to_id);

-- Simétrica: si A es amigo de B hay dos filas, (A, B) y (B, A).
CREATE TABLE friends (
    user_id    uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    friend_id  uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    since      timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, friend_id),
    CHECK (user_id <> friend_id)
);

-- La sala vive en memoria: la invitación guarda el código y vence a los 30 minutos.
CREATE TABLE room_invites (
    id          uuid PRIMARY KEY,
    from_id     uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    to_id       uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    room_code   text NOT NULL,
    created_at  timestamptz NOT NULL DEFAULT now(),
    UNIQUE (from_id, to_id, room_code)
);
CREATE INDEX room_invites_to ON room_invites(to_id, created_at DESC);
