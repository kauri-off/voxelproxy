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
            pub uuid: Option<Uuid>,
        }
    }

    pub mod game {
        use super::*;

        #[derive(Packet, Debug)]
        #[packet(0)] // ServerboundAcceptTeleportationPacket
        pub struct AcceptTeleportation {
            pub id: VarInt,
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
    }
}

pub mod s2c {
    use super::*;

    pub mod game {
        use super::*;

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
    }
}
