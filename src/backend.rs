use drax::transport::encryption::EncryptedWriter;
use drax::transport::frame::PacketFrame;
use drax::transport::DraxTransport;
use drax::VarInt;
use mcprotocol::chat;
use mcprotocol::chat::Chat;
use mcprotocol::pipeline::{AsyncMinecraftProtocolPipeline, MinecraftProtocolWriter};
use mcprotocol::prelude::Uuid;
use mcprotocol::protocol::login::MojangIdentifiedKey;
use mcprotocol::protocol::GameProfile;
use mcprotocol::registry::MappedAsyncPacketRegistry;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::tcp::OwnedReadHalf;
use tokio::net::tcp::OwnedWriteHalf;

use crate::cfg::ServerInfo;
use crate::ProxyInfo;
use crate::player::ClientInfo;

mod auth;
mod transition;

pub struct BackendContext {
    client_info: ClientInfo,
    client_write: MinecraftProtocolWriter<EncryptedWriter<OwnedWriteHalf>>,
    server_write: MinecraftProtocolWriter<OwnedWriteHalf>,
}

pub enum ForwardToServerType {
    ById(String),
    Info(ServerInfo),
}

pub enum EndpointResolution {
    DoNothing,
    DisconnectGracefully,
    ForwardToServer(ForwardToServerType),
}

pub struct BackendEndpoint {
    backend_context: BackendContext,
    server_read: AsyncMinecraftProtocolPipeline<
        OwnedReadHalf,
        BackendContext,
        EndpointResolution,
        MappedAsyncPacketRegistry<BackendContext, EndpointResolution>,
    >,
}

pub struct BackendEndpointWithNoContext {
    server_read: AsyncMinecraftProtocolPipeline<
        OwnedReadHalf,
        BackendContext,
        EndpointResolution,
        MappedAsyncPacketRegistry<BackendContext, EndpointResolution>,
    >,
    server_write: MinecraftProtocolWriter<OwnedWriteHalf>,
}

impl BackendEndpoint {
    pub async fn create_partial_connection(
        proxy_info: Arc<ProxyInfo>,
        server_info: &ServerInfo,
        client_info: &ClientInfo,
    ) -> anyhow::Result<BackendEndpointWithNoContext> {
        let auth::ConnectedServerBase { read, write, .. } =
            auth::connect_server_client(proxy_info, server_info, client_info).await?;
        Ok(BackendEndpointWithNoContext {
            server_read: read.clear_registry(),
            server_write: write,
        })
    }

    pub fn merge(
        old_server: BackendEndpoint,
        new_server: BackendEndpointWithNoContext,
    ) -> BackendEndpoint {
        let BackendEndpoint {
            backend_context, ..
        } = old_server;
        let BackendContext {
            client_info,
            client_write,
            ..
        } = backend_context;
        let BackendEndpointWithNoContext {
            server_read,
            server_write,
        } = new_server;
        BackendEndpoint {
            backend_context: BackendContext {
                client_info,
                client_write,
                server_write,
            },
            server_read,
        }
    }

    pub async fn read_next_server_packet_with_timeout(
        &mut self,
        timeout: Duration,
    ) -> Result<EndpointResolution, drax::transport::Error> {
        let endpoint_result = self
            .server_read
            .execute_next_packet_timeout(&mut self.backend_context, timeout)
            .await;
        match endpoint_result {
            Ok(resp) => Ok(resp),
            Err(err) => match err {
                mcprotocol::registry::RegistryError::NoHandlerFound(_, data) => {
                    self.backend_context
                        .client_write
                        .write_buffered_packet(PacketFrame { data })
                        .await?;
                    Ok(EndpointResolution::DoNothing)
                }
                mcprotocol::registry::RegistryError::DraxTransportError(err) => return Err(err),
            },
        }
    }
}
