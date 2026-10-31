-- Moderación: suspensión de cuentas y reportes de usuarios. Los administradores no se
-- guardan aquí: salen de ADMIN_EMAILS.

ALTER TABLE users
    ADD COLUMN banned_at timestamptz,
    ADD COLUMN ban_reason text;

CREATE TABLE reports (
    id           uuid PRIMARY KEY,
    reporter_id  uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    target_id    uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    reason       text NOT NULL CHECK (reason IN ('name', 'behavior', 'cheating', 'other')),
    details      text CHECK (char_length(details) <= 300),
    created_at   timestamptz NOT NULL DEFAULT now(),
    resolved_at  timestamptz,
    resolved_by  uuid REFERENCES users(id) ON DELETE SET NULL,
    -- Qué se hizo: dismiss (sin acción), rename o ban.
    action       text CHECK (action IN ('dismiss', 'rename', 'ban')),
    CHECK (reporter_id <> target_id)
);

-- Un solo reporte abierto de cada persona sobre otra.
CREATE UNIQUE INDEX reports_one_open ON reports(reporter_id, target_id) WHERE resolved_at IS NULL;
CREATE INDEX reports_open ON reports(created_at) WHERE resolved_at IS NULL;
