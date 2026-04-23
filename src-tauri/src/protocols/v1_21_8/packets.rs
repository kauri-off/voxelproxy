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
        use super::*;

        #[derive(Packet, Debug)]
        #[packet(3)] // ServerboundFinishConfigurationPacket
        pub struct FinishConfiguration {}
    }

    pub mod game {
        use super::*;

        #[derive(Packet, Debug)]
        #[packet(0)] // ServerboundAcceptTeleportationPacket
        pub struct AcceptTeleportation {
            pub id: VarInt,
        }

        #[derive(Packet, Debug)]
        #[packet(6)]
        pub struct ProtocolMetaDataSmall {
            pub data: String,
        }

        #[derive(Packet, Debug)]
        #[packet(8)]
        pub struct ProtocolMetaData {
            pub data: String,
            pub long: i64,
            pub long2: i64,
            pub option: Option<Vec<u8>>,
        }

        #[derive(Packet, Debug)]
        #[packet(29)] // ServerboundMovePlayerPacket.Pos
        pub struct Pos {
            pub x: f64,
            pub y: f64,
            pub z: f64,
            pub flags: u8,
        }

        #[derive(Packet, Debug)]
        #[packet(30)] // ServerboundMovePlayerPacket.PosRot
        pub struct PosRot {
            pub x: f64,
            pub y: f64,
            pub z: f64,
            pub yaw: f32,
            pub pitch: f32,
            pub flags: u8,
        }

        #[derive(Packet, Debug)]
        #[packet(31)] // ServerboundMovePlayerPacket.Rot
        pub struct Rot {
            pub yaw: f32,
            pub pitch: f32,
            pub flags: u8,
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
    }
}

pub mod s2c {
    use super::*;

    pub mod configuration {
        use super::*;

        #[derive(Packet, Debug)]
        #[packet(3)] // ClientboundFinishConfigurationPacket
        pub struct FinishConfiguration {}
    }

    pub mod game {
        use super::*;

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
    }
}
