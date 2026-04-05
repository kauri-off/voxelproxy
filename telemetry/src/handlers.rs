use axum::{
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use sqlx::SqlitePool;
use std::net::SocketAddr;

use crate::models::{AutoPayload, ManualPayload, PingPayload};
use crate::telegram;
use crate::utils::{extract_ip, now};

pub async fn handle_ping(
    State(db): State<SqlitePool>,
    headers: HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(payload): Json<PingPayload>,
) -> StatusCode {
    let ip = extract_ip(&headers, addr);
    let ts = now();

    let res =
        sqlx::query("INSERT INTO pings (ts, ip, version, os, username) VALUES (?, ?, ?, ?, ?)")
            .bind(&ts)
            .bind(&ip)
            .bind(&payload.version)
            .bind(&payload.os)
            .bind(&payload.username)
            .execute(&db)
            .await;

    match res {
        Ok(_) => {
            telegram::send(telegram::format_ping(
                &payload.username,
                &ip,
                &payload.version,
                &payload.os,
            ));
            StatusCode::OK
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

pub async fn handle_start_manual(
    State(db): State<SqlitePool>,
    headers: HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(payload): Json<ManualPayload>,
) -> StatusCode {
    let ip = extract_ip(&headers, addr);
    let ts = now();

    let res = sqlx::query("INSERT INTO starts (ts, ip, type, username, server_addr, windivert) VALUES (?, ?, 'manual', ?, ?, NULL)")
        .bind(&ts)
        .bind(&ip)
        .bind(&payload.username)
        .bind(&payload.server_addr)
        .execute(&db)
        .await;

    match res {
        Ok(_) => {
            telegram::send(telegram::format_manual(
                &payload.username,
                &payload.server_addr,
            ));
            StatusCode::OK
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

pub async fn handle_start_auto(
    State(db): State<SqlitePool>,
    headers: HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(payload): Json<AutoPayload>,
) -> StatusCode {
    let ip = extract_ip(&headers, addr);
    let ts = now();

    let res = sqlx::query("INSERT INTO starts (ts, ip, type, username, server_addr, windivert) VALUES (?, ?, 'auto', ?, NULL, ?)")
        .bind(&ts)
        .bind(&ip)
        .bind(&payload.username)
        .bind(payload.windivert)
        .execute(&db)
        .await;

    match res {
        Ok(_) => {
            telegram::send(telegram::format_auto(&payload.username, payload.windivert));
            StatusCode::OK
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
