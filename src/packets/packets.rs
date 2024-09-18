use std::io;

use minecraft_protocol::{
    types::var_int::VarInt,
    {packet_builder::PacketBuilder, packet_reader::PacketReader, UncompressedPacket},
};

/// PacketID 0x00
#[derive(Debug)]
pub struct Handshake {
    pub packet_id: VarInt,
    pub protocol_version: VarInt,
    pub server_address: String,
    pub server_port: u16,
    pub next_state: VarInt,
}

impl Handshake {
    pub fn serialize(self) -> UncompressedPacket {
        PacketBuilder::new(self.packet_id)
            .write_var_int(self.protocol_version)
            .write_string(self.server_address)
            .write_int(self.server_port)
            .write_var_int(self.next_state)
            .build()
    }

    pub async fn deserialize(packet: &UncompressedPacket) -> io::Result<Self> {
        let mut pr = PacketReader::new(packet);
        let protocol_version = pr.read_var_int().await?;
        let server_address = pr.read_string().await?;
        let server_port: u16 = pr.read_int()?;
        let next_state = pr.read_var_int().await?;

        Ok(Handshake {
            packet_id: packet.packet_id.clone(),
            protocol_version,
            server_address,
            server_port,
            next_state,
        })
    }
}

/// PacketID 0x00
#[derive(Clone)]
pub struct LoginStart {
    pub packet_id: VarInt,
    pub name: String,
    pub uuid: u128,
}

impl LoginStart {
    pub fn serialize(self) -> UncompressedPacket {
        PacketBuilder::new(self.packet_id)
            .write_string(self.name)
            .write_int(self.uuid)
            .build()
    }

    pub async fn deserialize(packet: &UncompressedPacket) -> io::Result<Self> {
        let mut packet_reader = PacketReader::new(packet);

        let name = packet_reader.read_string().await?;
        let uuid: u128 = packet_reader.read_int().unwrap_or(0);

        Ok(LoginStart {
            packet_id: packet.packet_id.clone(),
            name,
            uuid,
        })
    }
}

/// PacketID 0x03
#[derive(Clone, Debug)]
pub struct SetCompression {
    pub packet_id: VarInt,
    pub threshold: VarInt,
}

impl SetCompression {
    pub fn serialize(self) -> UncompressedPacket {
        PacketBuilder::new(self.packet_id)
            .write_var_int(self.threshold)
            .build()
    }

    pub async fn deserialize(packet: &UncompressedPacket) -> io::Result<Self> {
        let mut packet_reader = PacketReader::new(packet);

        Ok(SetCompression {
            packet_id: packet.packet_id.clone(),
            threshold: packet_reader.read_var_int().await?,
        })
    }
}

/// PacketID 0x03
pub struct ChatMessage {
    pub packet_id: VarInt,
    pub message: String,
}

impl ChatMessage {
    pub fn serialize(self) -> UncompressedPacket {
        PacketBuilder::new(self.packet_id)
            .write_string(self.message)
            .build()
    }

    pub async fn deserialize(packet: &UncompressedPacket) -> io::Result<Self> {
        let mut packet_reader = PacketReader::new(&packet);
        let packet_id = packet.packet_id.clone();

        let command = packet_reader.read_string().await?;

        Ok(ChatMessage {
            packet_id,
            message: command,
        })
    }
}

/// PacketID 0x00
pub struct Status {
    pub packet_id: VarInt,
    pub status: String,
}

impl Status {
    pub fn serialize(self) -> UncompressedPacket {
        PacketBuilder::new(self.packet_id.clone())
            .write_string(self.status)
            .build()
    }

    pub async fn deserialize(packet: &UncompressedPacket) -> io::Result<Self> {
        let mut packet_reader = PacketReader::new(packet);
        let packet_id = packet.packet_id.clone();

        let status = packet_reader.read_string().await?;

        Ok(Status { packet_id, status })
    }
}
