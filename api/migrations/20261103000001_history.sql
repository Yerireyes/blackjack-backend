-- Historial: las rondas liquidadas de un usuario, de la más reciente a la más antigua.
CREATE INDEX rounds_user_settled ON rounds(user_id, settled_at DESC) WHERE status = 'settled';
