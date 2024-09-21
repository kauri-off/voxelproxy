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

// PacketID 0x34
pub struct Position {
    pub packet_id: VarInt,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub yaw: f32,
    pub pitch: f32,
    pub flags: i8,
    pub teleportid: VarInt,
}

impl Position {
    pub fn serialize(self) -> UncompressedPacket {
        PacketBuilder::new(self.packet_id.clone())
            .write_int(self.x)
            .write_int(self.y)
            .write_int(self.z)
            .write_int(self.yaw)
            .write_int(self.pitch)
            .write_int(self.flags)
            .write_var_int(self.teleportid)
            .build()
    }

    pub async fn deserialize(packet: &UncompressedPacket) -> io::Result<Self> {
        let mut packet_reader = PacketReader::new(packet);
        let packet_id = packet.packet_id.clone();

        let x: f64 = packet_reader.read_int()?;
        let y: f64 = packet_reader.read_int()?;
        let z: f64 = packet_reader.read_int()?;
        let yaw: f32 = packet_reader.read_int()?;
        let pitch: f32 = packet_reader.read_int()?;
        let flags: i8 = packet_reader.read_int()?;
        let teleportid = packet_reader.read_var_int().await?;

        Ok(Position {
            packet_id,
            x,
            y,
            z,
            yaw,
            pitch,
            flags,
            teleportid,
        })
    }
}

// PacketID 0x34
#[derive(Debug)]
pub struct PositionLook {
    pub packet_id: VarInt,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub yaw: f32,
    pub pitch: f32,
    pub on_ground: bool,
}

impl PositionLook {
    pub fn serialize(self) -> UncompressedPacket {
        PacketBuilder::new(self.packet_id.clone())
            .write_int(self.x)
            .write_int(self.y)
            .write_int(self.z)
            .write_int(self.yaw)
            .write_int(self.pitch)
            .write_bool(self.on_ground)
            .build()
    }

    pub async fn deserialize(packet: &UncompressedPacket) -> io::Result<Self> {
        let mut packet_reader = PacketReader::new(packet);
        let packet_id = packet.packet_id.clone();

        let x: f64 = packet_reader.read_int()?;
        let y: f64 = packet_reader.read_int()?;
        let z: f64 = packet_reader.read_int()?;
        let yaw: f32 = packet_reader.read_int()?;
        let pitch: f32 = packet_reader.read_int()?;
        let on_ground = packet_reader.read_bool()?;

        Ok(PositionLook {
            packet_id,
            x,
            y,
            z,
            yaw,
            pitch,
            on_ground,
        })
    }
}
