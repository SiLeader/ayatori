FROM docker.io/library/rust:1.91.0-alpine AS builder

WORKDIR /work

RUN apk add musl-dev

COPY . .

RUN --mount=type=cache,target=/work/target \
    --mount=type=cache,target=/work/.cargo \
    cargo build --release && \
    cp target/release/ayatori /ayatori

FROM scratch

COPY --from=builder /ayatori /usr/local/bin/ayatori

CMD ["/usr/local/bin/ayatori"]
