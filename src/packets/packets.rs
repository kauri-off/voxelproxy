use std::io;

use minecraft_protocol::{
    types::var_int::VarInt,
    {packet_builder::PacketBuilder, packet_reader::PacketReader, UncompressedPacket},
};

pub mod c2s {
    use super::*;
    #[derive(Debug)]
    pub struct Handshake {
        pub protocol_version: VarInt,
        pub server_address: String,
        pub server_port: u16,
        pub next_state: VarInt,
    }

    impl Handshake {
        pub fn serialize(self) -> UncompressedPacket {
            PacketBuilder::new(VarInt(0x00))
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
                protocol_version,
                server_address,
                server_port,
                next_state,
            })
        }
    }
    #[derive(Clone)]
    pub struct LoginStart {
        pub name: String,
        pub uuid: u128,
    }

    impl LoginStart {
        pub fn serialize(self) -> UncompressedPacket {
            PacketBuilder::new(VarInt(0x00))
                .write_string(self.name)
                .write_int(self.uuid)
                .build()
        }

        pub async fn deserialize(packet: &UncompressedPacket) -> io::Result<Self> {
            let mut packet_reader = PacketReader::new(packet);

            let name = packet_reader.read_string().await?;
            let uuid: u128 = packet_reader.read_int().unwrap_or(0);

            Ok(LoginStart { name, uuid })
        }
    }

    pub struct ChatMessage {
        pub message: String,
    }

    impl ChatMessage {
        pub fn serialize(self) -> UncompressedPacket {
            PacketBuilder::new(VarInt(0x03))
                .write_string(self.message)
                .build()
        }

        pub async fn deserialize(packet: &UncompressedPacket) -> io::Result<Self> {
            let mut packet_reader = PacketReader::new(&packet);

            let message = packet_reader.read_string().await?;

            Ok(ChatMessage { message })
        }
    }

    #[derive(Debug)]
    pub struct Position {
        pub x: f64,
        pub y: f64,
        pub z: f64,
        pub on_ground: bool,
    }

    impl Position {
        pub fn serialize(self) -> UncompressedPacket {
            PacketBuilder::new(VarInt(0x12))
                .write_int(self.x)
                .write_int(self.y)
                .write_int(self.z)
                .write_bool(self.on_ground)
                .build()
        }

        pub async fn deserialize(packet: &UncompressedPacket) -> io::Result<Self> {
            let mut packet_reader = PacketReader::new(packet);

            let x: f64 = packet_reader.read_int()?;
            let y: f64 = packet_reader.read_int()?;
            let z: f64 = packet_reader.read_int()?;
            let on_ground = packet_reader.read_bool()?;

            Ok(Position { x, y, z, on_ground })
        }
    }

    #[derive(Debug)]
    pub struct PositionLook {
        pub x: f64,
        pub y: f64,
        pub z: f64,
        pub yaw: f32,
        pub pitch: f32,
        pub on_ground: bool,
    }

    impl PositionLook {
        pub fn serialize(self) -> UncompressedPacket {
            PacketBuilder::new(VarInt(0x13))
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

            let x: f64 = packet_reader.read_int()?;
            let y: f64 = packet_reader.read_int()?;
            let z: f64 = packet_reader.read_int()?;
            let yaw: f32 = packet_reader.read_int()?;
            let pitch: f32 = packet_reader.read_int()?;
            let on_ground = packet_reader.read_bool()?;

            Ok(PositionLook {
                x,
                y,
                z,
                yaw,
                pitch,
                on_ground,
            })
        }
    }

    #[derive(Debug)]
    pub struct Look {
        pub yaw: f32,
        pub pitch: f32,
        pub on_ground: bool,
    }

    impl Look {
        pub fn serialize(self) -> UncompressedPacket {
            PacketBuilder::new(VarInt(0x14))
                .write_int(self.yaw)
                .write_int(self.pitch)
                .write_bool(self.on_ground)
                .build()
        }

        pub async fn deserialize(packet: &UncompressedPacket) -> io::Result<Self> {
            let mut packet_reader = PacketReader::new(packet);

            let yaw: f32 = packet_reader.read_int()?;
            let pitch: f32 = packet_reader.read_int()?;
            let on_ground = packet_reader.read_bool()?;

            Ok(Look {
                yaw,
                pitch,
                on_ground,
            })
        }
    }
}

pub mod s2c {
    use super::*;

    #[derive(Clone)]
    pub struct SetCompression {
        pub threshold: VarInt,
    }

    impl SetCompression {
        pub fn serialize(self) -> UncompressedPacket {
            PacketBuilder::new(VarInt(0x03))
                .write_var_int(self.threshold)
                .build()
        }

        pub async fn deserialize(packet: &UncompressedPacket) -> io::Result<Self> {
            let mut packet_reader = PacketReader::new(packet);

            Ok(SetCompression {
                threshold: packet_reader.read_var_int().await?,
            })
        }
    }

    pub struct Status {
        pub status: String,
    }

    impl Status {
        pub fn serialize(self) -> UncompressedPacket {
            PacketBuilder::new(VarInt(0x00))
                .write_string(self.status)
                .build()
        }

        pub async fn deserialize(packet: &UncompressedPacket) -> io::Result<Self> {
            let mut packet_reader = PacketReader::new(packet);

            let status = packet_reader.read_string().await?;

            Ok(Status { status })
        }
    }
    #[derive(Clone)]
    pub struct Position {
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
            PacketBuilder::new(VarInt(0x34))
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

            let x: f64 = packet_reader.read_int()?;
            let y: f64 = packet_reader.read_int()?;
            let z: f64 = packet_reader.read_int()?;
            let yaw: f32 = packet_reader.read_int()?;
            let pitch: f32 = packet_reader.read_int()?;
            let flags: i8 = packet_reader.read_int()?;
            let teleportid = packet_reader.read_var_int().await?;

            Ok(Position {
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
}
