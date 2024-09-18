use crate::{types::{num::Integer, var_int::VarInt}, UncompressedPacket};

pub struct PacketBuilder {
    pub packet_id: VarInt,
    pub data: Vec<u8>,
}

impl PacketBuilder {
    pub fn new(packet_id: VarInt) -> PacketBuilder {
        PacketBuilder {
            packet_id,
            data: vec![],
        }
    }

    pub fn build(self) -> UncompressedPacket {
        UncompressedPacket {
            packet_id: self.packet_id,
            data: self.data,
        }
    }

    pub fn write_var_int(mut self, var_int: VarInt) -> Self {
        let _ = var_int.write_sync(&mut self.data);
        self
    }

    pub fn write_string(mut self, string: String) -> Self {
        self = self.write_var_int(VarInt(string.len() as i32));

        self.data.extend(string.as_bytes());
        self
    }

    pub fn write_int<I: Integer>(mut self, int: I) -> Self {
        self.data.extend(int.to_bytes());
        self
    }

    pub fn write_bool(self, b: bool) -> Self {
        match b {
            true => self.write_buffer(&[1]),
            false => self.write_buffer(&[0]),
        }
    }

    pub fn write_buffer(mut self, buf: &[u8]) -> Self {
        self.data.extend(buf);
        self
    }
}
