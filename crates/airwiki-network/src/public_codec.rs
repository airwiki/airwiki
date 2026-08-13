use std::io;
use std::marker::PhantomData;

use async_trait::async_trait;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use libp2p::StreamProtocol;
use libp2p::request_response::Codec;
use serde::{Serialize, de::DeserializeOwned};

pub(crate) trait ProtocolPayload {
    fn prepare_for_protocol(&mut self, protocol: &str) -> io::Result<()>;

    fn validate_protocol(&self, protocol: &str) -> io::Result<()>;
}

#[derive(Debug)]
pub(crate) struct VersionedCborCodec<Request, Response> {
    request_limit: u64,
    response_limit: u64,
    marker: PhantomData<(Request, Response)>,
}

impl<Request, Response> VersionedCborCodec<Request, Response> {
    pub(crate) fn new(request_limit: u64, response_limit: u64) -> Self {
        Self {
            request_limit,
            response_limit,
            marker: PhantomData,
        }
    }
}

impl<Request, Response> Clone for VersionedCborCodec<Request, Response> {
    fn clone(&self) -> Self {
        Self::new(self.request_limit, self.response_limit)
    }
}

#[async_trait]
impl<Request, Response> Codec for VersionedCborCodec<Request, Response>
where
    Request: Clone + DeserializeOwned + ProtocolPayload + Send + Serialize + Sync,
    Response: Clone + DeserializeOwned + ProtocolPayload + Send + Serialize + Sync,
{
    type Protocol = StreamProtocol;
    type Request = Request;
    type Response = Response;

    async fn read_request<T>(
        &mut self,
        protocol: &Self::Protocol,
        io: &mut T,
    ) -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send,
    {
        let request: Request = decode_bounded(io, self.request_limit).await?;
        request.validate_protocol(protocol.as_ref())?;
        Ok(request)
    }

    async fn read_response<T>(
        &mut self,
        protocol: &Self::Protocol,
        io: &mut T,
    ) -> io::Result<Self::Response>
    where
        T: AsyncRead + Unpin + Send,
    {
        let response: Response = decode_bounded(io, self.response_limit).await?;
        response.validate_protocol(protocol.as_ref())?;
        Ok(response)
    }

    async fn write_request<T>(
        &mut self,
        protocol: &Self::Protocol,
        io: &mut T,
        mut request: Self::Request,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        request.prepare_for_protocol(protocol.as_ref())?;
        encode_bounded(io, &request, self.request_limit).await
    }

    async fn write_response<T>(
        &mut self,
        protocol: &Self::Protocol,
        io: &mut T,
        mut response: Self::Response,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        response.prepare_for_protocol(protocol.as_ref())?;
        encode_bounded(io, &response, self.response_limit).await
    }
}

async fn decode_bounded<Value, Reader>(io: &mut Reader, limit: u64) -> io::Result<Value>
where
    Value: DeserializeOwned,
    Reader: AsyncRead + Unpin + Send,
{
    let mut bytes = Vec::new();
    io.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() as u64 > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "public CBOR payload exceeded its limit",
        ));
    }
    ciborium::from_reader(bytes.as_slice())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid public CBOR payload"))
}

async fn encode_bounded<T, W>(io: &mut W, value: &T, limit: u64) -> io::Result<()>
where
    T: Serialize,
    W: AsyncWrite + Unpin + Send,
{
    let mut bytes = Vec::new();
    ciborium::into_writer(value, &mut bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid public CBOR payload"))?;
    if bytes.len() as u64 > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "public CBOR payload exceeded its limit",
        ));
    }
    io.write_all(&bytes).await
}
