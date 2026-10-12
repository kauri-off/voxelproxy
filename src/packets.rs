pub mod universal {
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
}
