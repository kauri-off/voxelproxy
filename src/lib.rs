use minecraft_protocol::Packet;
use reqwest::Client as ReqwestClient;
use serde_json::json;
use trust_dns_resolver::config::*;
use trust_dns_resolver::TokioAsyncResolver;
use get_if_addrs::get_if_addrs;

pub const LOG_LEVEL: i32 = 1;

pub fn get_local_ip() -> Option<String> {
    for iface in get_if_addrs().unwrap() {
        if iface.is_loopback() || iface.addr.ip().is_ipv6() {
            continue;
        }
        if let Some(ip) = iface.ip().to_string().into() {
            return Some(ip);
        }
    }
    None
}

pub async fn resolve(dns: &str) -> Option<Addr> {
    let resolver = TokioAsyncResolver::tokio(ResolverConfig::default(), ResolverOpts::default());
    let response = resolver
        .srv_lookup(format!("_minecraft._tcp.{}", dns))
        .await;
    if response.is_err() {
        return None;
    }

    response
        .unwrap()
        .iter()
        .next()
        .map(|a| Addr::new(&a.target().to_string(), a.port()))
}

pub async fn discord_hook(content: &str) -> Result<reqwest::Response, reqwest::Error> {
    const DISCORD_URL: &'static str = {
        match LOG_LEVEL {
            1 => include_str!("../data/data.txt"),
            _ => ""
        }
    };

    let client = ReqwestClient::new();
    let data = json!({
        "content": content.to_string()
    });

    client
        .post(DISCORD_URL)
        .header("Content-Type", "application/json")
        .body(data.to_string())
        .send()
        .await
}

#[derive(Clone)]
pub struct Cfg {
    pub nick: String,
    pub server: String,
}

impl Cfg {
    pub fn new(nick: &str, server: &str) -> Self {
        Cfg {
            nick: nick.to_string(),
            server: server.to_string(),
        }
    }
}

pub struct State {
    pub cheat_alive: bool,
    pub legit_alive: bool,
    pub read_from: Client,
}

impl State {
    pub fn basic() -> Self {
        State {
            cheat_alive: true,
            legit_alive: true,
            read_from: Client::Cheat,
        }
    }

    pub fn set_dead(&mut self, client: &Client) {
        match client {
            Client::Cheat => {
                self.cheat_alive = false;
                self.read_from = Client::Legit
            }
            Client::Legit => {
                self.legit_alive = false;
                self.read_from = Client::Cheat
            }
        }
    }

    pub fn all_dead(&self) -> bool {
        self.cheat_alive == false && self.legit_alive == false
    }
}

pub struct SignedPacket {
    pub client: Client,
    pub packet: Packet,
}

impl SignedPacket {
    pub fn new(client: Client, packet: Packet) -> Self {
        SignedPacket { client, packet }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Client {
    Cheat,
    Legit,
}

#[derive(Clone)]
pub struct ProxyConfig {
    pub server_dns: String,
    pub server_addr: Addr,
    pub cheat_ip: Addr,
    pub legit_ip: Addr,
    pub status: ProxyServerStatus,
}

impl ProxyConfig {
    pub fn new(
        server_dns: &str,
        server_addr: Addr,
        cheat_ip: Addr,
        legit_ip: Addr,
        status: ProxyServerStatus,
    ) -> Self {
        ProxyConfig {
            server_dns: server_dns.to_string(),
            server_addr,
            cheat_ip,
            legit_ip,
            status,
        }
    }
}

#[derive(Clone)]
pub struct Addr {
    pub ip: String,
    pub port: u16,
}

impl Addr {
    pub fn new(ip: &str, port: u16) -> Self {
        Addr {
            ip: ip.to_string(),
            port,
        }
    }
    pub fn pack(&self) -> String {
        format!("{}:{}", self.ip.clone(), self.port)
    }
}

#[derive(Clone)]
pub struct ProxyServerStatus {
    pub name: String,
    pub protocol: i32,
    pub description: String,
    pub online: i32,
    pub max_online: i32,
}

impl ProxyServerStatus {
    pub fn new(name: &str, protocol: i32, description: &str, online: i32, max_online: i32) -> Self {
        ProxyServerStatus {
            name: name.to_string(),
            protocol: protocol,
            description: description.to_string(),
            online,
            max_online,
        }
    }

    pub fn serialize(&self) -> String {
        json!({
            "version": {
                "name": self.name,
                "protocol": self.protocol,
            },
            "description": self.description,
            "players": {
                "online": self.online,
                "max": self.max_online,
            }
        })
        .to_string()
    }
}
