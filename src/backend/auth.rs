use crate::player::ClientInfo;
use crate::cfg::{ForwardingMethod, ServerInfo};
use crate::ProxyInfo;
use mcprotocol::pipeline::{BlankAsyncProtocolPipeline, MinecraftProtocolWriter};
use mcprotocol::registry::RegistryError;
use std::sync::Arc;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;

#[derive(Debug, Clone)]
pub struct ServerStubInfo {
    pub server_id: Option<String>,
    pub server_name: String,
    pub server_ip: String,
    pub server_port: u16,
}

impl From<&ServerInfo> for ServerStubInfo {
    fn from(value: &ServerInfo) -> Self {
        ServerStubInfo {
            server_id: value.server_id.as_ref().cloned(),
            server_name: value.server_name.clone(),
            server_ip: value.server_ip.clone(),
            server_port: value.server_port,
        }
    }
}

pub struct ConnectedServerBase {
    pub info: ServerStubInfo,
    pub read: BlankAsyncProtocolPipeline<OwnedReadHalf>,
    pub write: MinecraftProtocolWriter<OwnedWriteHalf>,
}

pub async fn connect_server_client(
    proxy_info: Arc<ProxyInfo>,
    server: &ServerInfo,
    client: &ClientInfo,
) -> Result<ConnectedServerBase, RegistryError> {
    let connection = TcpStream::connect(format!("{}:{}", server.server_ip, server.server_port))
        .await
        .map_err(drax::transport::Error::TokioError)?;

    match server
        .forwarding
        .as_ref()
        .unwrap_or_else(|| &proxy_info.config.auth.default_forwarding)
    {
        ForwardingMethod::BungeeLegacy => {
            unimplemented!()
        }
        ForwardingMethod::VelocityModern { secret_key } => {
            let (read, write) =
                velocity::velocity_client_connect(server, connection, client, secret_key).await?;
            return Ok(ConnectedServerBase {
                info: ServerStubInfo::from(server),
                read,
                write,
            });
        }
    }
}

mod bungee {
    use crate::player::ClientInfo;
    use tokio::net::TcpStream;

    pub async fn bungee_client_connect(stream: TcpStream, client_info: &ClientInfo) {}
}

mod velocity {
    use crate::player::ClientInfo;
    use crate::cfg::ServerInfo;
    use drax::transport::{DraxTransport, TransportProcessorContext};
    use hmac::Hmac;
    use mcprotocol::pin_fut;
    use mcprotocol::pipeline::{
        buffer_packet, AsyncMinecraftProtocolPipeline, BlankAsyncProtocolPipeline,
        MinecraftProtocolWriter,
    };
    use mcprotocol::protocol::handshaking::sb::{Handshake, NextState};
    use mcprotocol::protocol::login::cb::LoginPluginRequest;
    use mcprotocol::protocol::login::sb::{LoginPluginResponse, LoginStart};
    use mcprotocol::protocol::GameProfile;
    use mcprotocol::registry::{RegistryError, UNKNOWN_VERSION};
    use sha2::digest::Mac;
    use sha2::Sha256;
    use std::cmp::{max, min};
    use std::io::Cursor;
    use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
    use tokio::net::TcpStream;

    type Hmac256 = Hmac<Sha256>;

    struct VelocityContext {
        hmac: Hmac256,
        sig_holder: Option<uuid::Uuid>,
    }

    pub async fn velocity_client_connect(
        server_info: &ServerInfo,
        stream: TcpStream,
        client_info: &ClientInfo,
        key: &String,
    ) -> Result<
        (
            BlankAsyncProtocolPipeline<OwnedReadHalf>,
            MinecraftProtocolWriter<OwnedWriteHalf>,
        ),
        RegistryError,
    > {
        let (read, write) = stream.into_split();

        let handshake = Handshake {
            protocol_version: client_info.protocol_version,
            server_address: server_info.server_ip.clone(),
            server_port: server_info.server_port,
            next_state: NextState::Login,
        };
        let buffered_handshake = buffer_packet(&handshake, UNKNOWN_VERSION)?;

        let login_start = LoginStart {
            name: client_info.profile.name.to_string(),
            sig_data: client_info.mojang_key.as_ref().cloned(),
            sig_holder: client_info.sig_holder.as_ref().cloned(),
        };

        let (mut read, mut write) = (
            AsyncMinecraftProtocolPipeline::from_protocol_version(
                read,
                client_info.protocol_version,
            ),
            MinecraftProtocolWriter::from_protocol_version(write, client_info.protocol_version),
        );
        write.write_buffered_packet(buffered_handshake).await?;
        write.write_packet(&login_start).await?;
        read.register(pin_fut!(handle_plugin_request));

        let hmac = Hmac256::new_from_slice(key.as_bytes()).expect("Hmac can be any length");

        let resp = read.execute_next_packet(&mut (hmac, client_info)).await??;
        write.write_packet(&resp).await?;
        Ok((read.clear_registry(), write))
    }

    async fn handle_plugin_request(
        ctx: &mut (Hmac256, &ClientInfo),
        request: LoginPluginRequest,
    ) -> Result<LoginPluginResponse, RegistryError> {
        let mask = if request.data.is_empty() {
            1
        } else {
            max(1, min(3, request.data[0]))
        };
        let mask = match (ctx.1.sig_holder.as_ref(), ctx.1.mojang_key.as_ref()) {
            (Some(_), Some(_)) => min(mask, 3),
            (Some(_), None) => min(mask, 2),
            _ => 1,
        };
        let mut data = Cursor::new(Vec::new());
        let mut tpx = TransportProcessorContext::new();
        drax::extension::write_var_int_sync(mask as i32, &mut tpx, &mut data)?;
        drax::extension::write_string(32767, &ctx.1.remote_addr.to_string(), &mut tpx, &mut data)?;
        GameProfile::write_to_transport(&ctx.1.profile, &mut tpx, &mut data)?;
        if mask > 1 {
            ctx.1
                .mojang_key
                .as_ref()
                .unwrap()
                .write_to_transport(&mut tpx, &mut data)?;
        }
        if mask > 2 {
            ctx.1
                .sig_holder
                .unwrap()
                .write_to_transport(&mut tpx, &mut data)?;
        }
        let data = data.into_inner();
        ctx.0.update(&data);
        let sig: Vec<u8> = ctx.0.clone().finalize().into_bytes().to_vec();

        Ok(LoginPluginResponse {
            message_id: request.message_id,
            successful: true,
            data: [sig, data].concat(),
        })
    }
}
