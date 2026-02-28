pub mod v1_16_5 {
    use mc_protocol::{Packet, varint::VarInt};

    pub mod c2s {

        use super::*;
        // ----------- HANDSHAKING -----------
        #[derive(Packet, Debug)]
        #[packet(0x00)]
        pub struct Handshake {
            pub protocol_version: VarInt,
            pub server_address: String,
            pub server_port: u16,
            pub intent: VarInt,
        }

        // ----------- STATUS -----------

        #[derive(Packet)]
        #[packet(0x00)]
        pub struct StatusRequest {}

        // ----------- LOGIN -----------
        #[derive(Packet, Debug)]
        #[packet(0x00)]
        pub struct LoginStart {
            pub name: String,
        }

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

        // ----------- STATUS -----------
        #[derive(Packet)]
        #[packet(0x00)]
        pub struct StatusResponse {
            pub response: String,
        }

        // ----------- LOGIN -----------
        #[derive(Packet, Debug)]
        #[packet(0x00)]
        pub struct LoginDisconnect {
            pub reason: String,
        }

        #[derive(Packet, Debug)]
        #[packet(0x03)]
        pub struct SetCompression {
            pub threshold: VarInt,
        }

        #[derive(Packet, Debug)]
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
}
