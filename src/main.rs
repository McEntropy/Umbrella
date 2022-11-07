#![feature(addr_parse_ascii)]

use mcprotocol::auth::AuthenticatedClient;
use mcprotocol::chat::Chat;
use mcprotocol::pin_fut;
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use crate::cfg::{IncomingAuthMethod, Players};
use mcprotocol::protocol::handshaking::sb::Handshake;
use mcprotocol::protocol::login::cb::Disconnect;
use mcprotocol::protocol::play::cb::KeepAlive;
use mcprotocol::protocol::status::cb::StatusResponsePlayers;
use mcprotocol::registry::RegistryError;
use mcprotocol::server_loop::{BaseConfiguration, IncomingAuthenticationOption, ServerLoop};
use mcprotocol::status::StatusBuilder;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpListener;
use tokio::sync::RwLock;

mod backend;
mod cfg;
mod client;
mod player;

pub struct MutableProxyInfo {
    pub current_players: i32,
}

pub type LockedProxyInfo = RwLock<MutableProxyInfo>;

pub struct ProxyInfo {
    pub mut_data: LockedProxyInfo,
    config: cfg::UmbrellaConfig,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config: cfg::UmbrellaConfig =
        serde_json::from_reader(fs::File::open(Path::new("./config.json"))?)?;

    let path = Path::new("./server-icon.png");
    let favicon = Arc::new(if path.exists() {
        let base_64 = image_base64::to_base64(path.to_str().unwrap());
        Some(base_64)
    } else {
        None
    });
    println!("{:?}", favicon);

    fern::Dispatch::new()
        .format(move |out, message, record| {
            out.finish(format_args!(
                "{} [{}/{}]: {}",
                chrono::Local::now().format("[%Y-%m-%d][%H:%M:%S]"),
                record.target(),
                record.level(),
                message
            ))
        })
        .level(config.log_level)
        .chain(std::io::stdout())
        .apply()?;

    log::info!("Umbrella logger initialized.");

    let proxy_info = Arc::new(ProxyInfo {
        mut_data: RwLock::new(MutableProxyInfo { current_players: 0 }),
        config,
    });

    let server_loop_proxy_info = proxy_info.clone();
    let server_loop = Arc::new(ServerLoop::new(
        {
            let (auth_option, auth_url) = match &proxy_info.config.auth.incoming_auth {
                IncomingAuthMethod::Mojang {
                    override_sessionserver,
                } => (
                    IncomingAuthenticationOption::MOJANG,
                    override_sessionserver.as_ref().cloned(),
                ),
                IncomingAuthMethod::BungeeLegacy => (IncomingAuthenticationOption::BUNGEE, None),
                IncomingAuthMethod::VelocityModern { secret_key } => (
                    IncomingAuthenticationOption::VELOCITY {
                        secret_key: secret_key.clone(),
                    },
                    None,
                ),
            };
            BaseConfiguration {
                auth_option,
                compression_threshold: proxy_info.config.compression_threshold,
                force_key_authentication: proxy_info.config.auth.force_key_authentication,
                auth_url,
            }
        },
        pin_fut!(wrapped_client_acceptor),
        move |h| {
            Box::pin(status_responder(
                server_loop_proxy_info.clone(),
                favicon.clone(),
                h,
            ))
        },
    ));

    log::info!(
        "Server loop successfully created. Binding to {}",
        proxy_info.config.bind
    );
    let listener = TcpListener::bind(&proxy_info.config.bind).await?;

    loop {
        let (stream, socket_addr) = listener.accept().await?;
        let loop_clone = server_loop.clone();
        let proxy_info = proxy_info.clone();
        tokio::spawn(async move {
            let (read, write) = stream.into_split();
            if let Err(registry_error) = ServerLoop::accept_client(
                loop_clone,
                ClientContext {
                    socket_addr,
                    proxy_info,
                },
                read,
                write,
            )
            .await
            {
                if !matches!(
                    registry_error,
                    RegistryError::DraxTransportError(
                        mcprotocol::prelude::drax::transport::Error::EOF
                    )
                ) {
                    log::warn!(
                        "Registry error encountered when accepting client: {}",
                        registry_error
                    );
                }
            }
        });
    }
}

async fn status_responder(
    proxy_info: Arc<ProxyInfo>,
    favicon: Arc<Option<String>>,
    _: Handshake,
) -> StatusBuilder {
    let no_lock = proxy_info.mut_data.read().await;
    let players = no_lock.current_players;
    drop(no_lock);

    let players = match &proxy_info.config.status.players {
        Players::Incremental => StatusResponsePlayers {
            max: players + 1,
            online: players,
            sample: vec![],
        },
        Players::Static {
            max_players,
            online_players,
        } => StatusResponsePlayers {
            max: *max_players,
            online: *online_players,
            sample: vec![],
        },
        Players::Capped { max_players } => StatusResponsePlayers {
            max: *max_players,
            online: players,
            sample: vec![],
        },
    };

    StatusBuilder {
        players,
        description: proxy_info.config.status.motd.clone(),
        favicon: (*favicon).as_ref().cloned(),
    }
}

pub struct ClientContext {
    socket_addr: SocketAddr,
    proxy_info: Arc<ProxyInfo>,
}

async fn wrapped_client_acceptor(
    context: ClientContext,
    mut rw: AuthenticatedClient<OwnedReadHalf, OwnedWriteHalf>,
) -> Result<(), RegistryError> {
    {
        let mut data_write = context.proxy_info.mut_data.write().await;
        if let Players::Capped { max_players } = context.proxy_info.config.status.players {
            if data_write.current_players >= max_players {
                rw.read_write
                    .1
                    .write_packet(&Disconnect {
                        reason: Chat::literal("Player limit reached."),
                    })
                    .await?;
                return Ok(());
            }
        }
        data_write.current_players += 1;
        drop(data_write);
    }
    let proxy_info_clone = context.proxy_info.clone();
    let ret = client_acceptor(context, rw).await;
    {
        let mut data_write = proxy_info_clone.mut_data.write().await;
        data_write.current_players -= 1;
        drop(data_write);
    }
    ret
}

async fn client_acceptor(
    mut context: ClientContext,
    mut rw: AuthenticatedClient<OwnedReadHalf, OwnedWriteHalf>,
) -> Result<(), RegistryError> {
    if let Some(overridden) = rw.overridden_address.as_ref() {
        context.socket_addr = SocketAddr::parse_ascii(overridden.as_bytes()).map_err(|err| {
            drax::transport::Error::Unknown(Some(format!("Failed to parse address {}", overridden)))
        })?;
    }
    rw.read_write.1.write_packet(&KeepAlive { id: 1 }).await?;
    // initial connect - setup a server connection (we should setup some entire client profile which handles everything)

    rw.read_write
        .1
        .write_packet(&Disconnect {
            reason: Chat::literal("kek"),
        })
        .await?;
    // todo: we have ourselves a bonified client
    Ok(())
}
