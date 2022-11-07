use log::LevelFilter;
use mcprotocol::chat::Chat;
use std::collections::HashMap;

#[derive(serde_derive::Deserialize, Debug)]
#[serde(tag = "auth_method", content = "auth_data")]
pub enum ForwardingMethod {
    #[serde(rename = "bungee")]
    BungeeLegacy,
    #[serde(rename = "velocity")]
    VelocityModern { secret_key: String },
}

#[derive(serde_derive::Deserialize, Debug)]
#[serde(tag = "auth_method", content = "auth_data")]
pub enum IncomingAuthMethod {
    #[serde(rename = "mojang")]
    Mojang {
        #[serde(skip_serializing_if = "Option::is_none")]
        override_sessionserver: Option<String>,
    },
    #[serde(rename = "bungee")]
    BungeeLegacy,
    #[serde(rename = "velocity")]
    VelocityModern { secret_key: String },
}

#[derive(serde_derive::Deserialize, Debug)]
pub struct ServerInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_id: Option<String>,
    pub server_name: String,
    pub server_ip: String,
    pub server_port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forwarding: Option<ForwardingMethod>,
}

#[derive(serde_derive::Deserialize, Debug)]
pub struct AuthConfig {
    pub force_key_authentication: bool,
    pub default_forwarding: ForwardingMethod,
    pub incoming_auth: IncomingAuthMethod,
}

#[derive(serde_derive::Deserialize, Debug)]
#[serde(untagged)]
pub enum Players {
    Incremental,
    Static {
        max_players: i32,
        online_players: i32,
    },
    Capped {
        max_players: i32,
    },
}

fn incremental() -> Players {
    Players::Incremental
}

#[derive(serde_derive::Deserialize, Debug)]
pub struct StatusConfig {
    pub motd: Chat,
    #[serde(default = "incremental")]
    pub players: Players,
}

#[derive(serde_derive::Deserialize, Debug)]
pub struct UmbrellaConfig {
    pub log_level: LevelFilter,
    pub bind: String,
    pub compression_threshold: isize,
    pub servers: HashMap<String, ServerInfo>,
    pub auth: AuthConfig,
    pub status: StatusConfig,
    pub fallback: Vec<String>,
    #[serde(rename = "try")]
    pub initial_try: Vec<String>,
}
