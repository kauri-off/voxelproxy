use mc_protocol::{Packet, varint::VarInt};

pub mod c2s {
    use super::*;

    #[derive(Packet, Debug)]
    #[packet(0x1C)]
    pub struct Pos {
        pub x: f64,
        pub y: f64,
        pub z: f64,
        pub on_ground: bool,
    }

    #[derive(Packet, Debug)]
    #[packet(0x1D)]
    pub struct PosRot {
        pub x: f64,
        pub y: f64,
        pub z: f64,
        pub yaw: f32,
        pub pitch: f32,
        pub on_ground: bool,
    }

    #[derive(Packet, Debug)]
    #[packet(0x1E)]
    pub struct Rot {
        pub yaw: f32,
        pub pitch: f32,
        pub on_ground: bool,
    }

    #[derive(Packet, Debug)]
    #[packet(0x2B)]
    pub struct Pong {
        pub id: i32,
    }
}

pub mod s2c {
    use super::*;

    #[derive(Packet, Debug)]
    #[packet(0x42)]
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
    #[packet(0x37)]
    pub struct Ping {
        pub id: i32,
    }
}
