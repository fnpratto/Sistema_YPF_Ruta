use serde::{Serialize, de::DeserializeOwned};
use std::io::{Error, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};

use crate::message::MsgType;

/*
 * TCP Protocol implementation for sending and receiving messages.
 *      - Messages are serialized as JSON strings.
 *      - Each message is prefixed with a 1-byte operation code (MsgType)
 *        followed by a 4-byte big-endian length of the message payload.
 */
pub struct TCPProtocol {
    stream: TcpStream,
}

impl TCPProtocol {
    pub async fn new(address: &str) -> Result<Self> {
        Ok(Self {
            stream: TcpStream::connect(address).await?,
        })
    }

    pub fn into_split(self) -> (OwnedReadHalf, OwnedWriteHalf) {
        self.stream.into_split()
    }

    fn serialize<T: Serialize>(msg: &T) -> Result<String> {
        serde_json::to_string(msg).map_err(|e| Error::other(format!("Serialization error: {}", e)))
    }

    fn deserialize<T: DeserializeOwned>(msg_str: &str) -> Result<T> {
        serde_json::from_str(msg_str)
            .map_err(|e| Error::other(format!("Deserialization error: {}", e)))
    }

    pub async fn send_with_writer<T: Serialize>(
        writer: &mut OwnedWriteHalf,
        msg_type: MsgType,
        msg: &T,
    ) -> Result<()> {
        let serialized = Self::serialize(msg)?;

        let op_code = msg_type as u8;
        let msg_len = serialized.len() as u32;
        let msg_bytes = serialized.as_bytes();

        writer.write_all(&[op_code]).await?;
        writer.write_all(&msg_len.to_be_bytes()).await?;
        writer.write_all(msg_bytes).await?;

        Ok(())
    }

    pub async fn receive_msg_type_from_reader(reader: &mut OwnedReadHalf) -> Result<MsgType> {
        let mut op_code_buffer = [0u8; 1];
        reader.read_exact(&mut op_code_buffer).await?;
        let op_code = op_code_buffer[0];

        MsgType::from_u8(op_code)
    }

    pub async fn receive_msg_from_reader<T: DeserializeOwned>(
        reader: &mut OwnedReadHalf,
    ) -> Result<T> {
        let mut len_buffer = [0u8; 4];
        reader.read_exact(&mut len_buffer).await?;
        let msg_len = u32::from_be_bytes(len_buffer) as usize;

        let mut buffer = vec![0u8; msg_len];
        reader.read_exact(&mut buffer).await?;
        let msg_str = String::from_utf8_lossy(&buffer);

        Self::deserialize::<T>(&msg_str)
    }
}
