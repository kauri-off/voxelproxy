use mc_protocol::{Packet, varint::VarInt};

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

        #[derive(Packet, Debug)]
        #[packet(4)] // ServerboundChatCommandPacket
        pub struct ChatCommand {
            pub command: String,
        }

        #[derive(Packet, Debug)]
        #[packet(6)] // ServerboundChatPacket
        pub struct Chat {
            pub message: String,
        }

        #[derive(Packet, Debug)]
        #[packet(12)] // ServerboundConfigurationAcknowledgedPacket
        pub struct ConfigurationAcknowledged {}

        #[derive(Packet, Debug)]
        #[packet(15)] // ServerboundContainerClosePacket
        pub struct ContainerClose {
            pub container_id: u8,
        }

        #[derive(Packet, Debug, Default)]
        #[packet(26)] // ServerboundMovePlayerPacket.Pos
        pub struct Pos {
            pub x: f64,
            pub y: f64,
            pub z: f64,
            pub on_ground: bool,
        }

        #[derive(Packet, Debug, Default)]
        #[packet(27)] // ServerboundMovePlayerPacket.PosRot
        pub struct PosRot {
            pub x: f64,
            pub y: f64,
            pub z: f64,
            pub yaw: f32,
            pub pitch: f32,
            pub on_ground: bool,
        }

        #[derive(Packet, Debug, Default)]
        #[packet(28)] // ServerboundMovePlayerPacket.Rot
        pub struct Rot {
            pub yaw: f32,
            pub pitch: f32,
            pub on_ground: bool,
        }

        #[derive(Packet, Debug)]
        #[packet(8)] // ServerboundChunkBatchReceivedPacket
        pub struct ChunkBatchReceived {
            pub desired_chunks_per_tick: f32,
        }

        #[derive(Packet, Debug)]
        #[packet(39)] // ServerboundPongPacket
        pub struct Pong {
            pub id: i32,
        }

        #[derive(Packet, Debug)]
        #[packet(24)] // ServerboundKeepAlivePacket
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
        #[packet(9)] // ClientboundResourcePackPushPacket
        pub struct ResourcePackPush {
            pub id: Uuid,
        }

        #[derive(Packet, Debug)]
        #[packet(14)] // ClientboundSelectKnownPacks
        pub struct SelectKnownPacks {}
    }

    pub mod game {
        use super::*;

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
        #[packet(53)] // ClientboundPingPacket
        pub struct Ping {
            pub id: i32,
        }

        #[derive(Packet, Debug)]
        #[packet(64)] // ClientboundPlayerPositionPacket
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
        #[packet(105)] // ClientboundStartConfigurationPacket
        pub struct StartConfiguration {}

        #[derive(Packet, Debug)]
        #[packet(38)] // ClientboundKeepAlivePacket
        pub struct KeepAlive {
            pub id: i64,
        }
    }
}
