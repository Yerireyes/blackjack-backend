# Blackjack en un contenedor: la API y, si se pasa el código del front, también el front
# compilado, servido por la misma API (mismo origen: cookie, WebSocket y Google sin CORS).
# Las migraciones van dentro del binario y se aplican al arrancar.
#
# Con el front (lo que se despliega):
#   docker build --build-context frontend-src=<carpeta del repo del front> -t blackjack .
# Solo la API (la CI de este repo):
#   docker build -t blackjack .
#
#   docker run --env-file .env -p 8080:8080 blackjack

# Código del front. Vacío por defecto; --build-context frontend-src=... lo reemplaza.
FROM scratch AS frontend-src

# Base de Rust del motor WASM y de la API.
FROM rust:1-bookworm AS rust
WORKDIR /w/Backend
ENV SQLX_OFFLINE=true

# Motor WASM para el front (@bj/engine-wasm, que el front enlaza desde ../Backend).
FROM rust AS wasm
COPY --from=node:24-bookworm-slim /usr/local/bin/node /usr/local/bin/node
RUN rustup target add wasm32-unknown-unknown \
    && cargo install wasm-pack --version 0.15.0 --locked
COPY . .
RUN node scripts/build-wasm.mjs

FROM node:24-bookworm-slim AS front
WORKDIR /w/Frontend
COPY --from=wasm /w/Backend/engine-wasm/pkg /w/Backend/engine-wasm/pkg
COPY --from=frontend-src / ./
# Sin front (solo API) queda un dist/ vacío y la API no sirve nada fuera de /api/v1.
RUN if [ -f package.json ]; then \
        rm -rf node_modules dist && npm ci --no-audit --no-fund && npm run build; \
    else \
        mkdir -p dist; \
    fi

FROM rust AS api
COPY . .
RUN cargo build --release --locked -p api

FROM debian:bookworm-slim
# Certificados raíz para hablar con Google por HTTPS.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=api /w/Backend/target/release/api /usr/local/bin/api
COPY --from=front /w/Frontend/dist /srv/www
# PORT lo pisa la plataforma (Render, Fly, Koyeb) si inyecta otro; la API escucha en
# 0.0.0.0:$PORT cuando no hay BIND_ADDR.
ENV PORT=8080 \
    STATIC_DIR=/srv/www \
    RUST_LOG=api=info,tower_http=info
EXPOSE 8080
USER 10001:10001
CMD ["api"]
