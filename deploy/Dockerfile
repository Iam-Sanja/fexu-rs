FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --locked --release -p myshop --bin myshop \
    && cargo build --locked --release -p fexu-cli --bin fexu

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends wget ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 fexu
WORKDIR /home/fexu
COPY --from=build /src/target/release/myshop /usr/local/bin/myshop
COPY --from=build /src/target/release/fexu /usr/local/bin/fexu
COPY --from=build /src/examples/myshop/static /home/fexu/static
COPY --from=build /src/examples/myshop/migrations /home/fexu/migrations
RUN mkdir -p /home/fexu/media && chown fexu:fexu /home/fexu/media
USER fexu
ENV BIND_ADDRESS=0.0.0.0:8080
ENV FEXU_ALLOW_WILDCARD_BIND=true
ENV FEXU_MYSHOP_MEDIA_DIR=/home/fexu/media
ENV FEXU_MYSHOP_STATIC_DIR=/home/fexu/static
EXPOSE 8080
HEALTHCHECK --interval=10s --timeout=3s --retries=5 CMD wget -qO- http://127.0.0.1:8080/readyz || exit 1
CMD ["myshop"]
