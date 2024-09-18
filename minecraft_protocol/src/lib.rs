use std::{
    fmt::Debug,
    io::{self, Cursor, Read, Write},
};

use types::var_int::VarInt;
use flate2::{bufread::ZlibDecoder, write::ZlibEncoder, Compression};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub mod types;
pub mod packet_builder;
pub mod packet_reader;

#[derive(Clone)]
pub enum Packet {
    UnCompressed(UncompressedPacket),
    Compressed(CompressedPacket),
}

impl Debug for Packet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Packet::UnCompressed(p) => write!(
                f,
                "UnCompressed, PacketID: 0x{:x}, Len: {}",
                p.packet_id.0,
                p.data.len()
            ),
            Packet::Compressed(p) => write!(f, "Comressed, Data len: {}", p.body_len.0),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CompressedPacket {
    pub body_len: VarInt,
    pub body: Vec<u8>, // PacketID + Data
}

impl CompressedPacket {
    /// BodyLen + Body(PacketID + Data)
    pub async fn pack(&self) -> io::Result<Vec<u8>> {
        let mut body = Vec::new();
        self.body_len.write(&mut body).await?;
        body.extend(&self.body);

        Ok(body)
    }

    /// Body(PacketID + Data) => Packet(packet_id, data)
    pub async fn decompress(&self) -> io::Result<UncompressedPacket> {
        let data = Packet::decompress_data(&self.body).await?;
        let mut stream = &data[..];
        UncompressedPacket::unpack(&mut stream).await
    }

    /// PacketLen + Body(data_len + data(packet_id + data)) => Stream
    pub async fn write<W: AsyncWrite + Unpin>(&self, writer: &mut W) -> io::Result<()> {
        let body = self.pack().await?;
        VarInt(body.len() as i32).write(writer).await?;
        writer.write_all(&body).await
    }
}

#[derive(Debug, Clone)]
pub struct UncompressedPacket {
    pub packet_id: VarInt,
    pub data: Vec<u8>,
}

impl UncompressedPacket {
    /// Body(PacketID + data) => packet(packet_id, data)
    pub async fn unpack<R: AsyncRead + Unpin>(body: &mut R) -> io::Result<Self> {
        let packet_id = VarInt::read(body).await?;
        let mut data = Vec::new();
        body.read_to_end(&mut data).await?;

        Ok(UncompressedPacket { packet_id, data })
    }

    /// packet(packet_id, data) => Body(PacketID + data)
    pub async fn pack(&self) -> io::Result<Vec<u8>> {
        let mut body = Vec::new();
        self.packet_id.write(&mut body).await?;
        body.extend(&self.data);

        Ok(body)
    }

    /// packet(packet_id, data) => CompressedPacket
    pub async fn compress(&self, threshold: i32) -> io::Result<CompressedPacket> {
        let body = self.pack().await?;

        if (body.len() as i32) < threshold {
            Ok(CompressedPacket {
                body_len: VarInt(0),
                body,
            })
        } else {
            let len = body.len();
            let body = Packet::compress_data(&body).await?;
            Ok(CompressedPacket {
                body_len: VarInt(len as i32),
                body,
            })
        }
    }

    /// PacketLen + Body(packet_id + data) => Stream
    pub async fn write<W: AsyncWrite + Unpin>(&self, writer: &mut W) -> io::Result<()> {
        let body = self.pack().await?;

        VarInt(body.len() as i32).write(writer).await?;
        writer.write_all(&body).await
    }
}

impl Packet {
    pub async fn read<R: AsyncRead + Unpin>(
        reader: &mut R,
        threshold: Option<i32>,
    ) -> io::Result<Self> {
        match threshold {
            Some(threshold) => Packet::read_compressed(reader, threshold).await,
            None => Ok(Self::UnCompressed(Packet::read_uncompressed(reader).await?)),
        }
    }

    pub async fn read_uncompressed<R: AsyncRead + Unpin>(
        reader: &mut R,
    ) -> io::Result<UncompressedPacket> {
        let body = Packet::read_body(reader).await?;
        let mut stream = &body[..];

        UncompressedPacket::unpack(&mut stream).await
    }

    pub async fn read_compressed<R: AsyncRead + Unpin>(
        reader: &mut R,
        _threshold: i32,
    ) -> io::Result<Self> {
        let body = Packet::read_body(reader).await?;
        let mut stream = &body[..];

        let data_length = VarInt::read(&mut stream).await?;

        match data_length.0 {
            0 => Ok(Self::UnCompressed(
                UncompressedPacket::unpack(&mut stream).await?,
            )),
            _ => Ok(Self::Compressed(CompressedPacket {
                body_len: data_length,
                body: stream.to_vec(),
            })),
        }
    }

    /// Read packet_len then read packet body
    pub async fn read_body<R: AsyncRead + Unpin>(reader: &mut R) -> io::Result<Vec<u8>> {
        let length = VarInt::read(reader).await?;
        let mut body = vec![0; length.0 as usize];

        reader.read_exact(&mut body).await?;
        Ok(body)
    }

    pub async fn write<W: AsyncWrite + Unpin>(
        self: &Self,
        writer: &mut W,
        threshold: Option<i32>,
    ) -> io::Result<()> {
        match self {
            Packet::UnCompressed(uncompressed) => match threshold {
                Some(t) => uncompressed.compress(t).await?.write(writer).await,
                None => uncompressed.write(writer).await,
            },
            Packet::Compressed(compressed) => compressed.write(writer).await,
        }
    }

    pub async fn packet_id(&self) -> io::Result<VarInt> {
        match self {
            Packet::UnCompressed(uncompressed) => Ok(uncompressed.packet_id.clone()),
            Packet::Compressed(compressed) => Ok(compressed.decompress().await?.packet_id),
        }
    }

    pub async fn compress_data(data: &[u8]) -> io::Result<Vec<u8>> {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(6));
        encoder.write_all(data)?;
        let compressed_data = encoder.finish()?;
        Ok(compressed_data)
    }

    pub async fn decompress_data(data: &[u8]) -> io::Result<Vec<u8>> {
        let mut decoder = ZlibDecoder::new(Cursor::new(data));
        let mut decompressed_data = Vec::new();
        decoder.read_to_end(&mut decompressed_data)?;
        Ok(decompressed_data)
    }
}
