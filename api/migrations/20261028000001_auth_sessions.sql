-- Sesiones de login revocables. El JWT de la cookie lleva el id de su fila (`sid`)
-- y solo vale mientras la fila exista, no esté revocada y no haya vencido.
-- (`sessions` ya es la tabla de las sesiones de práctica.)
--
-- Los JWT emitidos antes de esta migración no tienen `sid` y dejan de valer: hay
-- que volver a iniciar sesión.

CREATE TABLE auth_sessions (
    id           uuid PRIMARY KEY,
    user_id      uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at   timestamptz NOT NULL DEFAULT now(),
    expires_at   timestamptz NOT NULL,
    revoked_at   timestamptz,
    -- Se actualiza como mucho cada 5 minutos para no escribir en cada petición.
    last_seen_at timestamptz NOT NULL DEFAULT now(),
    user_agent   text
);

CREATE INDEX auth_sessions_user_id ON auth_sessions(user_id);
CREATE INDEX auth_sessions_expires_at ON auth_sessions(expires_at);
