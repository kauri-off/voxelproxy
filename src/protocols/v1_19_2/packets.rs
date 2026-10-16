use mc_protocol::{Packet, ser::ByteArray, varint::VarInt};

pub mod c2s {
    use super::*;

    pub mod login {
        use super::*;

        #[derive(Packet, Debug)]
        #[packet(0)] // ServerboundHelloPacket
        pub struct HelloPacket {
            pub name: String,
        }
    }

    pub mod game {
        use uuid::Uuid;

        use super::*;

        #[derive(Packet, Debug)]
        #[packet(0)] // ServerboundAcceptTeleportationPacket
        pub struct AcceptTeleportation {
            pub id: VarInt,
        }

        /// `ArgumentSignatures.Entry`
        #[derive(Packet, Debug)]
        pub struct ArgumentSignature {
            pub name: String,
            pub signature: ByteArray,
        }

        /// `LastSeenMessages.Entry`
        #[derive(Packet, Debug)]
        pub struct LastSeenEntry {
            pub profile_id: Uuid,
            pub signature: ByteArray,
        }

        /// `LastSeenMessages.Update`
        #[derive(Packet, Debug)]
        pub struct LastSeen {
            pub last_seen: Vec<LastSeenEntry>,
            pub last_received: Option<LastSeenEntry>,
        }

        #[derive(Packet, Debug)]
        #[packet(4)] // ServerboundChatCommandPacket
        pub struct ChatCommand {
            pub command: String,
            pub time_stamp: i64,
            pub salt: i64,
            pub argument_signatures: Vec<ArgumentSignature>,
            pub signed_preview: bool,
            pub last_seen: LastSeen,
        }

        #[derive(Packet, Debug)]
        #[packet(5)] // ServerboundChatPacket
        pub struct Chat {
            pub message: String,
            pub time_stamp: i64,
            pub salt: i64,
            pub signature: ByteArray,
            pub signed_preview: bool,
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
        use super::*;

        #[derive(Packet, Debug)]
        #[packet(57)] // ClientboundPlayerPositionPacket
        pub struct Position {
            pub x: f64,
            pub y: f64,
            pub z: f64,
            pub yaw: f32,
            pub pitch: f32,
            pub relative_flags: u8,
            pub id: VarInt,
            pub dismount_vehicle: bool,
        }

        #[derive(Packet, Debug)]
        #[packet(47)] // ClientboundPingPacket
        pub struct Ping {
            pub id: i32,
        }

        #[derive(Packet, Debug)]
        #[packet(16)] // ClientboundContainerClosePacket
        pub struct ContainerClose {
            pub container_id: u8,
        }

        #[derive(Packet, Debug)]
        #[packet(32)] // ClientboundKeepAlivePacket
        pub struct KeepAlive {
            pub id: i64,
        }
    }
}
