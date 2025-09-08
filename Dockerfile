FROM rust:1.56.0-alpine as builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
RUN cargo build --release

FROM debian:latest
RUN apt-get update && apt-get install -y sqlite3 openssl curl
COPY --from=builder /app/target/release/volatile-proxy /usr/local/bin/
CMD ["/usr/local/bin/volatile-proxy"]
EXPOSE 9050
