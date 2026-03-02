use mc_protocol::{Packet, varint::VarInt};

pub mod c2s {
    use super::*;

    #[derive(Packet, Debug)]
    #[packet(0x14)]
    pub struct Look {
        pub yaw: f32,
        pub pitch: f32,
        pub on_ground: bool,
    }

    #[derive(Packet, Debug)]
    #[packet(0x13)]
    pub struct PositionLook {
        pub x: f64,
        pub y: f64,
        pub z: f64,
        pub yaw: f32,
        pub pitch: f32,
        pub on_ground: bool,
    }

    #[derive(Packet, Debug)]
    #[packet(0x12)]
    pub struct Position {
        pub x: f64,
        pub y: f64,
        pub z: f64,
        pub on_ground: bool,
    }

    #[derive(Packet, Debug, Clone)]
    #[packet(0x07)]
    pub struct Transaction {
        pub window_id: i8,
        pub action: i16,
        pub accepted: bool,
    }
}

pub mod s2c {
    use super::*;

    #[derive(Packet, Debug, Clone)]
    #[packet(0x34)]
    pub struct Position {
        pub x: f64,
        pub y: f64,
        pub z: f64,
        pub yaw: f32,
        pub pitch: f32,
        pub flags: i8,
        pub teleport_id: VarInt,
    }

    #[derive(Packet, Debug, Clone)]
    #[packet(0x11)]
    pub struct Transaction {
        pub window_id: i8,
        pub action: i16,
        pub accepted: bool,
    }
}
