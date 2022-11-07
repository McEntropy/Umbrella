use std::{future::Future, time::Duration};

use mcprotocol::{
    pipeline::AsyncMinecraftProtocolPipeline,
    registry::{AsyncPacketRegistry, MappedAsyncPacketRegistry, RegistryError},
};
use tokio::net::tcp::OwnedReadHalf;

pub enum ClientFunctionResponse {
    DoNothing,
    ForwardPacket(Vec<u8>),
    ForwardPackets(Vec<Vec<u8>>),
}

pub struct Client {
    read: AsyncMinecraftProtocolPipeline<
        OwnedReadHalf,
        (), /* todo */
        ClientFunctionResponse,
        MappedAsyncPacketRegistry<() /* todo */, ClientFunctionResponse>,
    >,
}

impl Client {
    pub fn create<
        _1: Send + Sync,
        _2: Send + Sync,
        Reg: AsyncPacketRegistry<_1, _2> + Send + Sync,
    >(
        current_pipeline: AsyncMinecraftProtocolPipeline<OwnedReadHalf, _1, _2, Reg>,
    ) -> Client {
        let pipeline = current_pipeline.clear_registry();
        Client { read: pipeline }
    }

    pub async fn read_next_packet_with_timeout(
        &mut self,
        timeout: Duration,
    ) -> Result<ClientFunctionResponse, drax::transport::Error> {
        match (&mut self.read)
            .execute_next_packet_timeout(&mut (), timeout)
            .await
        {
            Ok(resp) => Ok(resp),
            Err(registry_error) => match registry_error {
                RegistryError::NoHandlerFound(_, data) => {
                    Ok(ClientFunctionResponse::ForwardPacket(data))
                }
                RegistryError::DraxTransportError(err) => return Err(err),
            },
        }
    }
}
