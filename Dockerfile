FROM rust:latest

RUN apt-get update && \
    apt-get install -y mingw-w64 && \
    rustup target add x86_64-pc-windows-gnu

WORKDIR /usr/src/myapp

COPY minecraft_protocol minecraft_protocol
COPY src src
COPY Cargo.toml .
COPY data data

# Компилируем проект для Windows
CMD [ "cargo", "build", "--release", "--target", "x86_64-pc-windows-gnu", "--target-dir=/win" ]