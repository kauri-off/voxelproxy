use std::io::{self, Error, Read, Write};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const SEGMENT_BITS: i32 = 0x7F;
const CONTINUE_BIT: i32 = 0x80;

/// https://wiki.vg/Protocol#VarInt_and_VarLong
#[derive(Clone, Debug)]
pub struct VarInt(pub i32);

impl VarInt {
    pub async fn read<R: AsyncRead + Unpin>(reader: &mut R) -> io::Result<Self> {
        let mut value: i32 = 0;
        let mut position: i32 = 0;

        loop {
            let current_byte = reader.read_u8().await? as i32;

            value |= (current_byte & SEGMENT_BITS) << position;

            if (current_byte & CONTINUE_BIT) == 0 {
                break;
            }
            position += 7;

            if position >= 32 {
                return Err(Error::new(io::ErrorKind::InvalidData, "Position >= 32"));
            }
        }

        Ok(VarInt(value))
    }

    pub fn read_sync<R: Read + Unpin>(reader: &mut R) -> io::Result<Self> {
        let mut value: i32 = 0;
        let mut position: i32 = 0;

        loop {
            let mut buf = [0; 1];
            reader.read(&mut buf)?;
            let current_byte = buf[0] as i32;

            value |= (current_byte & SEGMENT_BITS) << position;

            if (current_byte & CONTINUE_BIT) == 0 {
                break;
            }
            position += 7;

            if position >= 32 {
                return Err(Error::new(io::ErrorKind::InvalidData, "Position >= 32"));
            }
        }

        Ok(VarInt(value))
    }

    pub async fn write<W: AsyncWrite + Unpin>(self: &Self, writer: &mut W) -> io::Result<()> {
        let mut value = self.0;
        loop {
            if (value & !SEGMENT_BITS) == 0 {
                writer.write_u8(value as u8).await?;
                break;
            }

            writer
                .write_u8(((value & SEGMENT_BITS) | CONTINUE_BIT) as u8)
                .await?;

            value = ((value as u32) >> 7) as i32;
        }

        Ok(())
    }

    pub fn write_sync<W: Write + Unpin>(self: &Self, writer: &mut W) -> io::Result<()> {
        let mut value = self.0;
        loop {
            if (value & !SEGMENT_BITS) == 0 {
                writer.write(&[value as u8])?;
                break;
            }

            writer.write(&[((value & SEGMENT_BITS) | CONTINUE_BIT) as u8])?;

            value = ((value as u32) >> 7) as i32;
        }

        Ok(())
    }
}
