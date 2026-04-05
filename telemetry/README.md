# Telemetry

## Описание

Сервер для сбора телеметрии с HTTP API и сохранением в SQLite.

Поддерживает опциональную отправку событий в Telegram через compile-time feature.

## Telegram feature

Отправка сообщений в Telegram включается только при сборке с feature:

```
cargo build --features telegram
```

Без этой feature код Telegram полностью отключен и вызовы становятся no-op.

## Настройка Telegram

Перед сборкой с feature `telegram` необходимо создать файлы:

- `TG_TOKEN` — токен бота
- `TG_ADMIN_ID` — chat id

Они читаются через `include_str!`.

## Запуск

```
cargo run
```

С Telegram:

```
cargo run --features telegram
```

## API

```
POST /voxelproxy/v1/ping

POST /voxelproxy/v1/start_manual

POST /voxelproxy/v1/start_auto
```
