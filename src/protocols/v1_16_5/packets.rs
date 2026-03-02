use mc_protocol::{Packet, varint::VarInt};

pub mod c2s {
    use super::*;

    pub mod game {
        use super::*;

        #[derive(Packet, Debug)]
        #[packet(0)] // ServerboundAcceptTeleportationPacket
        pub struct AcceptTeleportation {
            pub id: VarInt,
        }

        #[derive(Packet, Debug)]
        #[packet(18)] // ServerboundMovePlayerPacket.Pos
        pub struct Pos {
            pub x: f64,
            pub y: f64,
            pub z: f64,
            pub on_ground: bool,
        }

        #[derive(Packet, Debug)]
        #[packet(19)] // ServerboundMovePlayerPacket.PosRot
        pub struct PosRot {
            pub x: f64,
            pub y: f64,
            pub z: f64,
            pub yaw: f32,
            pub pitch: f32,
            pub on_ground: bool,
        }

        #[derive(Packet, Debug)]
        #[packet(20)] // ServerboundMovePlayerPacket.Rot
        pub struct Rot {
            pub yaw: f32,
            pub pitch: f32,
            pub on_ground: bool,
        }

        #[derive(Packet, Debug, Clone)]
        #[packet(7)] // ServerboundContainerAckPacket
        pub struct Ack {
            pub container_id: i8,
            pub uid: i16,
            pub accepted: bool,
        }
    }
}

pub mod s2c {
    use super::*;
    pub mod game {
        use super::*;

        #[derive(Packet, Debug, Clone)]
        #[packet(52)] // ClientboundPlayerPositionPacket
        pub struct Position {
            pub x: f64,
            pub y: f64,
            pub z: f64,
            pub yaw: f32,
            pub pitch: f32,
            pub relative_flags: u8,
            pub id: VarInt,
        }

        #[derive(Packet, Debug, Clone)]
        #[packet(17)] // ClientboundContainerAckPacket
        pub struct ContainerAck {
            pub container_id: i8,
            pub uid: i16,
            pub accepted: bool,
        }
    }
}
