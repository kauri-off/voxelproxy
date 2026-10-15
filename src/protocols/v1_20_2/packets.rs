use mc_protocol::{Packet, varint::VarInt};

use crate::protocols::Fixed;

pub mod c2s {
    use super::*;

    pub mod login {
        use uuid::Uuid;

        use super::*;

        #[derive(Packet, Debug)]
        #[packet(0)] // ServerboundHelloPacket
        pub struct HelloPacket {
            pub name: String,
            pub uuid: Uuid,
        }

        #[derive(Packet, Debug)]
        #[packet(3)] // ServerboundLoginAcknowledgedPacket
        pub struct LoginAcknowledged {}
    }

    pub mod configuration {
        use super::*;

        #[derive(Packet, Debug)]
        #[packet(2)] // ServerboundFinishConfigurationPacket
        pub struct FinishConfiguration {}

        #[derive(Packet, Debug)]
        #[packet(3)] // ServerboundKeepAlivePacket
        pub struct KeepAlive {
            pub id: i64,
        }

        #[derive(Packet, Debug)]
        #[packet(4)] // ServerboundPongPacket
        pub struct Pong {
            pub id: i32,
        }

        #[derive(Packet, Debug)]
        #[packet(5)] // ServerboundResourcePackPacket
        pub struct ResourcePack {
            pub action: VarInt,
        }
    }

    pub mod game {
        use super::*;

        #[derive(Packet, Debug)]
        #[packet(0)] // ServerboundAcceptTeleportationPacket
        pub struct AcceptTeleportation {
            pub id: VarInt,
        }

        /// `LastSeenMessages.Update`
        #[derive(Packet, Debug, Clone, Copy, PartialEq)]
        pub struct LastSeen {
            pub offset: VarInt,
            pub acknowledged: Fixed<3>,
        }

        /// `ArgumentSignatures.Entry`
        #[derive(Packet, Debug)]
        pub struct ArgumentSignature {
            pub name: String,
            pub signature: Fixed<256>,
        }

        #[derive(Packet, Debug)]
        #[packet(3)] // ServerboundChatAckPacket
        pub struct ChatAck {
            pub offset: VarInt,
        }

        #[derive(Packet, Debug)]
        #[packet(4)] // ServerboundChatCommandPacket
        pub struct ChatCommand {
            pub command: String,
            pub time_stamp: i64,
            pub salt: i64,
            pub argument_signatures: Vec<ArgumentSignature>,
            pub last_seen: LastSeen,
        }

        #[derive(Packet, Debug)]
        #[packet(5)] // ServerboundChatPacket
        pub struct Chat {
            pub message: String,
            pub time_stamp: i64,
            pub salt: i64,
            pub signature: Option<Fixed<256>>,
            pub last_seen: LastSeen,
        }

        #[derive(Packet, Debug)]
        #[packet(11)] // ServerboundConfigurationAcknowledgedPacket
        pub struct ConfigurationAcknowledged {}

        #[derive(Packet, Debug)]
        #[packet(14)] // ServerboundContainerClosePacket
        pub struct ContainerClose {
            pub container_id: u8,
        }

        #[derive(Packet, Debug, Default)]
        #[packet(22)] // ServerboundMovePlayerPacket.Pos
        pub struct Pos {
            pub x: f64,
            pub y: f64,
            pub z: f64,
            pub on_ground: bool,
        }

        #[derive(Packet, Debug, Default)]
        #[packet(23)] // ServerboundMovePlayerPacket.PosRot
        pub struct PosRot {
            pub x: f64,
            pub y: f64,
            pub z: f64,
            pub yaw: f32,
            pub pitch: f32,
            pub on_ground: bool,
        }

        #[derive(Packet, Debug, Default)]
        #[packet(24)] // ServerboundMovePlayerPacket.Rot
        pub struct Rot {
            pub yaw: f32,
            pub pitch: f32,
            pub on_ground: bool,
        }

        #[derive(Packet, Debug)]
        #[packet(7)] // ServerboundChunkBatchReceivedPacket
        pub struct ChunkBatchReceived {
            pub desired_chunks_per_tick: f32,
        }

        #[derive(Packet, Debug)]
        #[packet(35)] // ServerboundPongPacket
        pub struct Pong {
            pub id: i32,
        }

        #[derive(Packet, Debug)]
        #[packet(20)] // ServerboundKeepAlivePacket
        pub struct KeepAlive {
            pub id: i64,
        }
    }
}

pub mod s2c {
    use super::*;

    pub mod configuration {
        use super::*;

        #[derive(Packet, Debug)]
        #[packet(2)] // ClientboundFinishConfigurationPacket
        pub struct FinishConfiguration {}

        #[derive(Packet, Debug)]
        #[packet(3)] // ClientboundKeepAlivePacket
        pub struct KeepAlive {
            pub id: i64,
        }

        #[derive(Packet, Debug)]
        #[packet(4)] // ClientboundPingPacket
        pub struct Ping {
            pub id: i32,
        }

        #[derive(Packet, Debug)]
        #[packet(6)] // ClientboundResourcePackPacket
        pub struct ResourcePack {}
    }

    pub mod game {
        use uuid::Uuid;

        use super::*;

        #[derive(Packet, Debug)]
        #[packet(55)] // ClientboundPlayerChatPacket
        pub struct PlayerChat {
            pub sender: Uuid,
            pub index: VarInt,
            pub signature: Option<Fixed<256>>,
        }

        #[derive(Packet, Debug)]
        #[packet(18)] // ClientboundContainerClosePacket
        pub struct ContainerClose {
            pub container_id: u8,
        }

        #[derive(Packet, Debug)]
        #[packet(12)] // ClientboundChunkBatchFinishedPacket
        pub struct ChunkBatchFinished {
            pub batch_size: VarInt,
        }

        #[derive(Packet, Debug)]
        #[packet(51)] // ClientboundPingPacket
        pub struct Ping {
            pub id: i32,
        }

        #[derive(Packet, Debug)]
        #[packet(62)] // ClientboundPlayerPositionPacket
        pub struct Position {
            pub x: f64,
            pub y: f64,
            pub z: f64,
            pub yaw: f32,
            pub pitch: f32,
            pub relative_flags: u8,
            pub id: VarInt,
        }

        #[derive(Packet, Debug)]
        #[packet(101)] // ClientboundStartConfigurationPacket
        pub struct StartConfiguration {}

        #[derive(Packet, Debug)]
        #[packet(36)] // ClientboundKeepAlivePacket
        pub struct KeepAlive {
            pub id: i64,
        }
    }
}
