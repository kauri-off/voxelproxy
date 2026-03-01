pub mod v1_16_5 {
    use mc_protocol::{Packet, varint::VarInt};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[repr(i32)]
    pub enum Intent {
        Status = 1,
        Login = 2,
    }

    impl TryFrom<i32> for Intent {
        type Error = ();

        fn try_from(value: i32) -> Result<Self, Self::Error> {
            match value {
                1 => Ok(Intent::Status),
                2 => Ok(Intent::Login),
                _ => Err(()),
            }
        }
    }

    impl From<Intent> for VarInt {
        fn from(intent: Intent) -> Self {
            VarInt(intent as i32)
        }
    }

    pub mod handshaking {
        use super::*;

        pub mod c2s {
            use super::*;

            #[derive(Packet, Debug)]
            #[packet(0x00)]
            pub struct Handshake {
                pub protocol_version: VarInt,
                pub server_address: String,
                pub server_port: u16,
                pub intent: VarInt,
            }
        }
    }

    pub mod status {
        use super::*;

        pub mod c2s {
            use super::*;

            #[derive(Packet)]
            #[packet(0x00)]
            pub struct StatusRequest {}
        }

        pub mod s2c {
            use super::*;

            #[derive(Packet)]
            #[packet(0x00)]
            pub struct StatusResponse {
                pub response: String,
            }
        }
    }

    pub mod login {
        use super::*;

        pub mod c2s {
            use super::*;

            #[derive(Packet, Debug)]
            #[packet(0x00)]
            pub struct LoginStart {
                pub name: String,
            }
        }

        pub mod s2c {
            use super::*;

            #[derive(Packet, Debug)]
            #[packet(0x00)]
            pub struct LoginDisconnect {
                pub reason: String,
            }

            #[derive(Packet)]
            #[packet(0x01)]
            pub struct EncryptionRequest {}

            #[derive(Packet)]
            #[packet(0x02)]
            pub struct LoginSuccess {}

            #[derive(Packet, Debug)]
            #[packet(0x03)]
            pub struct SetCompression {
                pub threshold: VarInt,
            }
        }
    }

    pub mod play {
        use super::*;

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
}
