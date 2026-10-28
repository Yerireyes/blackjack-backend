-- Prueba de reautenticación para vincular Google: `POST /auth/google/link-intent`
-- (con la contraseña) la deja en la sesión por unos minutos, y
-- `GET /auth/google/start?link=1` la consume (un solo uso).
ALTER TABLE auth_sessions ADD COLUMN link_intent_until timestamptz;
