use mc_protocol::{Packet, varint::VarInt};

pub mod c2s {
    use super::*;

    #[derive(Packet, Debug)]
    #[packet(28)] // ServerboundMovePlayerPacket.Pos
    pub struct Pos {
        pub x: f64,
        pub y: f64,
        pub z: f64,
        pub on_ground: bool,
    }

    #[derive(Packet, Debug)]
    #[packet(29)] // ServerboundMovePlayerPacket.PosRot
    pub struct PosRot {
        pub x: f64,
        pub y: f64,
        pub z: f64,
        pub yaw: f32,
        pub pitch: f32,
        pub on_ground: bool,
    }

    #[derive(Packet, Debug)]
    #[packet(30)] // ServerboundMovePlayerPacket.Rot
    pub struct Rot {
        pub yaw: f32,
        pub pitch: f32,
        pub on_ground: bool,
    }

    #[derive(Packet, Debug)]
    #[packet(43)] // ServerboundPongPacket
    pub struct Pong {
        pub id: i32,
    }
}

pub mod s2c {
    use super::*;

    #[derive(Packet, Debug)]
    #[packet(66)] // ClientboundPlayerPositionPacket
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
    #[packet(55)] // ClientboundPingPacket
    pub struct Ping {
        pub id: i32,
    }
}
