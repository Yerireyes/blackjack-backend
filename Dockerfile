# API de Blackjack en un contenedor. Compila con los datos de .sqlx (sin base) y deja
# solo el binario en una imagen mínima. Las migraciones van dentro del binario y se
# aplican al arrancar. El front (dist/) lo sirve el reverse proxy, no este contenedor.
#
#   docker build -t blackjack-api .
#   docker run --env-file .env -p 8080:8080 blackjack-api

FROM rust:1-bookworm AS build
WORKDIR /src
ENV SQLX_OFFLINE=true
COPY . .
RUN cargo build --release --locked -p api

FROM debian:bookworm-slim
# Certificados raíz para hablar con Google por HTTPS.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/api /usr/local/bin/api
# Dentro del contenedor hay que escuchar en todas las interfaces; el reverse proxy de la
# máquina conecta a 127.0.0.1:8080 publicado por docker.
ENV BIND_ADDR=0.0.0.0:8080 \
    RUST_LOG=api=info,tower_http=info
EXPOSE 8080
USER 10001:10001
CMD ["api"]
