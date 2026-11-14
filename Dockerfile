# One image for tradoor-server, tradoor-recorder and tradoor-backfill, with the
# web app built in and served by nginx or the server.
#
#   docker build -t tradoor:latest .
#
# Dependencies compile in their own layer (cargo-chef), so a change to tradoor's
# code rebuilds in minutes when that layer is cached, as it is in CI. Written for
# the classic builder too, so it needs no BuildKit features.

FROM denoland/deno:bin-2.9.6 AS deno

FROM rust:1.95-slim-trixie AS toolchain
ARG WASM_BINDGEN=0.2.128
ARG BINARYEN=version_132
ARG CARGO_CHEF=0.1.78
# cmake and a C++ compiler build aws-lc for rustls; curl fetches the WASM tools.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl cmake g++ make pkg-config \
 && rm -rf /var/lib/apt/lists/*
RUN rustup target add wasm32-unknown-unknown \
 && curl -fsSL "https://github.com/wasm-bindgen/wasm-bindgen/releases/download/${WASM_BINDGEN}/wasm-bindgen-${WASM_BINDGEN}-x86_64-unknown-linux-musl.tar.gz" \
    | tar -xz -C /usr/local/bin --strip-components=1 "wasm-bindgen-${WASM_BINDGEN}-x86_64-unknown-linux-musl/wasm-bindgen" \
 && curl -fsSL "https://github.com/WebAssembly/binaryen/releases/download/${BINARYEN}/binaryen-${BINARYEN}-x86_64-linux.tar.gz" \
    | tar -xz -C /usr/local/bin --strip-components=2 "binaryen-${BINARYEN}/bin/wasm-opt"
RUN cargo install cargo-chef --version "${CARGO_CHEF}" --locked
COPY --from=deno /deno /usr/local/bin/deno
WORKDIR /src

FROM toolchain AS planner
COPY Cargo.toml Cargo.lock ./
COPY crates crates
COPY bins bins
RUN cargo chef prepare --recipe-path recipe.json

FROM toolchain AS build
# Dependencies only: reused until Cargo.lock or a manifest changes.
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release --locked --recipe-path recipe.json -p tradoor-server -p tradoor-recorder -p tradoor-backfill \
 && cargo chef cook --profile wasm --locked --target wasm32-unknown-unknown --recipe-path recipe.json -p tradoor-wasm
COPY web/package.json web/deno.json web/deno.lock web/
RUN cd web && deno install
COPY Cargo.toml Cargo.lock ./
COPY crates crates
COPY bins bins
COPY migrations migrations
COPY scripts scripts
COPY web web
RUN cargo build --release --locked -p tradoor-server -p tradoor-recorder -p tradoor-backfill
RUN ./scripts/build-wasm.sh
# The site's public address, for its canonical URL, preview image and sitemap (web/site.ts).
ARG SITE_URL=""
RUN cd web && SITE_URL="${SITE_URL}" deno task build

FROM debian:trixie-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --user-group --home-dir /nonexistent --shell /usr/sbin/nologin tradoor \
 && mkdir /data && chown tradoor:tradoor /data
COPY --from=build /src/target/release/tradoor-server /src/target/release/tradoor-recorder /src/target/release/tradoor-backfill /usr/local/bin/
COPY --from=build /src/web/dist /app/web
ENV TRADOOR_BIND=0.0.0.0:8080 \
    TRADOOR_DATA_DIR=/data \
    TRADOOR_WEB_DIR=/app/web
USER tradoor
VOLUME /data
EXPOSE 8080
CMD ["tradoor-server"]
