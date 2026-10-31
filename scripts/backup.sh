#!/usr/bin/env sh
# Respaldo de PostgreSQL en el servidor de producción (Linux), pensado para cron:
#
#   0 4 * * * /srv/blackjack/scripts/backup.sh >> /var/log/blackjack-backup.log 2>&1
#
# Con docker compose (por defecto) usa el pg_dump del contenedor `db`; con
# BACKUP_MODE=direct usa el pg_dump de la máquina y DATABASE_URL.
#
# Variables: BACKUP_DIR (por defecto ./backups), BACKUP_KEEP_DAYS (14), BACKUP_MODE
# (compose | direct), DATABASE_URL (solo direct).
#
# Restaurar (borra lo que haya en la base):
#   docker compose exec -T db pg_restore -U blackjack -d blackjack --clean --if-exists < archivo.dump
#
# Las salas multijugador viven en memoria y no se respaldan (es lo esperado).
set -eu

dir="${BACKUP_DIR:-$(dirname "$0")/../backups}"
keep="${BACKUP_KEEP_DAYS:-14}"
mode="${BACKUP_MODE:-compose}"
mkdir -p "$dir"
file="$dir/blackjack-$(date -u +%Y%m%d-%H%M%S).dump"

case "$mode" in
  compose)
    cd "$(dirname "$0")/.."
    docker compose exec -T db pg_dump -U blackjack -d blackjack --format=custom > "$file"
    ;;
  direct)
    : "${DATABASE_URL:?falta DATABASE_URL}"
    pg_dump "$DATABASE_URL" --format=custom --file="$file"
    ;;
  *)
    echo "BACKUP_MODE debe ser compose o direct" >&2
    exit 1
    ;;
esac

# Un respaldo vacío es un error, no un respaldo.
if [ ! -s "$file" ]; then
  echo "el respaldo quedó vacío: $file" >&2
  rm -f "$file"
  exit 1
fi
echo "respaldo: $file ($(du -h "$file" | cut -f1))"

# Borra los de más de BACKUP_KEEP_DAYS días.
find "$dir" -name 'blackjack-*.dump' -type f -mtime +"$keep" -print -delete
