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
        use uuid::Uuid;

        use super::*;

        #[derive(Packet, Debug)]
        #[packet(3)] // ServerboundFinishConfigurationPacket
        pub struct FinishConfiguration {}

        #[derive(Packet, Debug)]
        #[packet(4)] // ServerboundKeepAlivePacket
        pub struct KeepAlive {
            pub id: i64,
        }

        #[derive(Packet, Debug)]
        #[packet(5)] // ServerboundPongPacket
        pub struct Pong {
            pub id: i32,
        }

        #[derive(Packet, Debug)]
        #[packet(6)] // ServerboundResourcePackPacket
        pub struct ResourcePack {
            pub id: Uuid,
            pub action: VarInt,
        }

        #[derive(Packet, Debug)]
        #[packet(7)] // ServerboundSelectKnownPacks
        pub struct SelectKnownPacks {}
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
            pub checksum: u8,
        }

        /// `ArgumentSignatures.Entry`
        #[derive(Packet, Debug)]
        pub struct ArgumentSignature {
            pub name: String,
            pub signature: Fixed<256>,
        }

        #[derive(Packet, Debug)]
        #[packet(5)] // ServerboundChatAckPacket
        pub struct ChatAck {
            pub offset: VarInt,
        }

        #[derive(Packet, Debug)]
        #[packet(6)] // ServerboundChatCommandPacket
        pub struct ChatCommand {
            pub command: String,
        }

        #[derive(Packet, Debug)]
        #[packet(7)] // ServerboundChatCommandSignedPacket
        pub struct ChatCommandSigned {
            pub command: String,
            pub time_stamp: i64,
            pub salt: i64,
            pub argument_signatures: Vec<ArgumentSignature>,
            pub last_seen: LastSeen,
        }

        #[derive(Packet, Debug)]
        #[packet(8)] // ServerboundChatPacket
        pub struct Chat {
            pub message: String,
            pub time_stamp: i64,
            pub salt: i64,
            pub signature: Option<Fixed<256>>,
            pub last_seen: LastSeen,
        }

        #[derive(Packet, Debug)]
        #[packet(15)] // ServerboundConfigurationAcknowledgedPacket
        pub struct ConfigurationAcknowledged {}

        #[derive(Packet, Debug, Default)]
        #[packet(29)] // ServerboundMovePlayerPacket.Pos
        pub struct Pos {
            pub x: f64,
            pub y: f64,
            pub z: f64,
            pub flags: u8,
        }

        #[derive(Packet, Debug, Default)]
        #[packet(30)] // ServerboundMovePlayerPacket.PosRot
        pub struct PosRot {
            pub x: f64,
            pub y: f64,
            pub z: f64,
            pub yaw: f32,
            pub pitch: f32,
            pub flags: u8,
        }

        #[derive(Packet, Debug, Default)]
        #[packet(31)] // ServerboundMovePlayerPacket.Rot
        pub struct Rot {
            pub yaw: f32,
            pub pitch: f32,
            pub flags: u8,
        }

        #[derive(Packet, Debug)]
        #[packet(10)] // ServerboundChunkBatchReceivedPacket
        pub struct ChunkBatchReceived {
            pub desired_chunks_per_tick: f32,
        }

        #[derive(Packet, Debug)]
        #[packet(44)] // ServerboundPongPacket
        pub struct Pong {
            pub id: i32,
        }

        #[derive(Packet, Debug)]
        #[packet(18)] // ServerboundContainerClosePacket
        pub struct ContainerClose {
            pub container_id: VarInt,
        }

        #[derive(Packet, Debug)]
        #[packet(27)] // ServerboundKeepAlivePacket
        pub struct KeepAlive {
            pub id: i64,
        }
    }
}

pub mod s2c {
    use super::*;

    pub mod configuration {
        use uuid::Uuid;

        use super::*;

        #[derive(Packet, Debug)]
        #[packet(3)] // ClientboundFinishConfigurationPacket
        pub struct FinishConfiguration {}

        #[derive(Packet, Debug)]
        #[packet(4)] // ClientboundKeepAlivePacket
        pub struct KeepAlive {
            pub id: i64,
        }

        #[derive(Packet, Debug)]
        #[packet(5)] // ClientboundPingPacket
        pub struct Ping {
            pub id: i32,
        }

        #[derive(Packet, Debug)]
        #[packet(9)] // ClientboundResourcePackPushPacket
        pub struct ResourcePackPush {
            pub id: Uuid,
        }

        #[derive(Packet, Debug)]
        #[packet(14)] // ClientboundSelectKnownPacks
        pub struct SelectKnownPacks {}
    }

    pub mod game {
        use uuid::Uuid;

        use super::*;

        #[derive(Packet, Debug)]
        #[packet(58)] // ClientboundPlayerChatPacket
        pub struct PlayerChat {
            pub global_index: VarInt,
            pub sender: Uuid,
            pub index: VarInt,
            pub signature: Option<Fixed<256>>,
        }

        #[derive(Packet, Debug)]
        #[packet(65)] // ClientboundPlayerPositionPacket
        pub struct Position {
            pub id: VarInt,
            pub x: f64,
            pub y: f64,
            pub z: f64,
            pub delta_x: f64,
            pub delta_y: f64,
            pub delta_z: f64,
            pub yaw: f32,
            pub pitch: f32,
            pub relative_flags: i32,
        }

        #[derive(Packet, Debug)]
        #[packet(11)] // ClientboundChunkBatchFinishedPacket
        pub struct ChunkBatchFinished {
            pub batch_size: VarInt,
        }

        #[derive(Packet, Debug)]
        #[packet(54)] // ClientboundPingPacket
        pub struct Ping {
            pub id: i32,
        }

        #[derive(Packet, Debug)]
        #[packet(111)] // ClientboundStartConfigurationPacket
        pub struct StartConfiguration {}

        #[derive(Packet, Debug)]
        #[packet(17)] // ClientboundContainerClosePacket
        pub struct ContainerClose {
            pub container_id: VarInt,
        }

        #[derive(Packet, Debug)]
        #[packet(38)] // ClientboundKeepAlivePacket
        pub struct KeepAlive {
            pub id: i64,
        }
    }
}
