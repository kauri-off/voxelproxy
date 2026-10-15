use mc_protocol::{Packet, varint::VarInt};

use crate::protocols::Fixed;

pub mod c2s {
    use super::*;

    pub mod login {
        use super::*;
        use uuid::Uuid;

        #[derive(Packet, Debug)]
        #[packet(0)] // ServerboundHelloPacket
        pub struct HelloPacket {
            pub name: String,
            pub uuid: Option<Uuid>,
        }
    }

    pub mod game {
        use super::*;

        #[derive(Packet, Debug)]
        #[packet(13)] // ServerboundCustomPayloadPacket
        pub struct CustomPayload {
            pub channel: String,
        }

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
        #[packet(20)] // ServerboundMovePlayerPacket.Pos
        pub struct Pos {
            pub x: f64,
            pub y: f64,
            pub z: f64,
            pub on_ground: bool,
        }

        #[derive(Packet, Debug)]
        #[packet(21)] // ServerboundMovePlayerPacket.PosRot
        pub struct PosRot {
            pub x: f64,
            pub y: f64,
            pub z: f64,
            pub yaw: f32,
            pub pitch: f32,
            pub on_ground: bool,
        }

        #[derive(Packet, Debug)]
        #[packet(22)] // ServerboundMovePlayerPacket.Rot
        pub struct Rot {
            pub yaw: f32,
            pub pitch: f32,
            pub on_ground: bool,
        }

        #[derive(Packet, Debug)]
        #[packet(32)] // ServerboundPongPacket
        pub struct Pong {
            pub id: i32,
        }

        #[derive(Packet, Debug)]
        #[packet(12)] // ServerboundContainerClosePacket
        pub struct ContainerClose {
            pub container_id: u8,
        }

        #[derive(Packet, Debug)]
        #[packet(18)] // ServerboundKeepAlivePacket
        pub struct KeepAlive {
            pub id: i64,
        }
    }
}

pub mod s2c {
    use super::*;

    pub mod game {
        use uuid::Uuid;

        use super::*;

        #[derive(Packet, Debug)]
        #[packet(40)] // ClientboundLoginPacket
        pub struct Login {}

        #[derive(Packet, Debug)]
        #[packet(53)] // ClientboundPlayerChatPacket
        pub struct PlayerChat {
            pub sender: Uuid,
            pub index: VarInt,
            pub signature: Option<Fixed<256>>,
        }

        #[derive(Packet, Debug)]
        #[packet(60)] // ClientboundPlayerPositionPacket
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
        #[packet(50)] // ClientboundPingPacket
        pub struct Ping {
            pub id: i32,
        }

        #[derive(Packet, Debug)]
        #[packet(17)] // ClientboundContainerClosePacket
        pub struct ContainerClose {
            pub container_id: u8,
        }

        #[derive(Packet, Debug)]
        #[packet(35)] // ClientboundKeepAlivePacket
        pub struct KeepAlive {
            pub id: i64,
        }
    }
}
