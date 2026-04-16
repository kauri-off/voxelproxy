use serde::Deserialize;

#[derive(Deserialize)]
pub struct PingPayload {
    pub version: String,
    pub os: String,
    pub username: String,
}

#[derive(Deserialize)]
pub struct ManualPayload {
    pub username: String,
    pub server_addr: String,
}

#[derive(Deserialize)]
pub struct AutoPayload {
    pub username: String,
    pub windivert: bool,
}

#[derive(Deserialize)]
pub struct AutoJoin {
    pub username: String,
    pub server_addr: String,
}
