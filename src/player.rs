use std::{net::SocketAddr, sync::Arc};

use drax::VarInt;
use mcprotocol::protocol::{login::MojangIdentifiedKey, GameProfile};
use uuid::Uuid;

use crate::{backend::BackendEndpoint, client::Client, ProxyInfo};

#[derive(Clone, Debug)]
pub struct ClientInfo {
    pub protocol_version: VarInt,
    pub remote_addr: SocketAddr,
    pub mojang_key: Option<MojangIdentifiedKey>,
    pub sig_holder: Option<Uuid>,
    pub profile: GameProfile,
}

pub struct ConnectedPlayer {
    client_info: ClientInfo,
    proxy_info: Arc<ProxyInfo>,
    client: Client,
    backend_endpoint: BackendEndpoint,
}

impl ConnectedPlayer {}
