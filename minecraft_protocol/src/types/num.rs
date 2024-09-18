pub trait Integer {
    fn to_bytes(&self) -> Vec<u8>;

    fn from_bytes(bytes: &[u8]) -> Self;

    fn byte_len() -> usize;
}

impl Integer for i8 {
    fn to_bytes(&self) -> Vec<u8> {
        self.to_be_bytes().to_vec()
    }

    fn from_bytes(bytes: &[u8]) -> Self {
        i8::from_be_bytes(bytes.try_into().unwrap())
    }

    fn byte_len() -> usize {
        std::mem::size_of::<Self>()
    }
}

impl Integer for i16 {
    fn to_bytes(&self) -> Vec<u8> {
        self.to_be_bytes().to_vec()
    }

    fn from_bytes(bytes: &[u8]) -> Self {
        i16::from_be_bytes(bytes.try_into().unwrap())
    }

    fn byte_len() -> usize {
        std::mem::size_of::<Self>()
    }
}

impl Integer for i32 {
    fn to_bytes(&self) -> Vec<u8> {
        self.to_be_bytes().to_vec()
    }

    fn from_bytes(bytes: &[u8]) -> Self {
        i32::from_be_bytes(bytes.try_into().unwrap())
    }

    fn byte_len() -> usize {
        std::mem::size_of::<Self>()
    }
}

impl Integer for i64 {
    fn to_bytes(&self) -> Vec<u8> {
        self.to_be_bytes().to_vec()
    }

    fn from_bytes(bytes: &[u8]) -> Self {
        i64::from_be_bytes(bytes.try_into().unwrap())
    }

    fn byte_len() -> usize {
        std::mem::size_of::<Self>()
    }
}

impl Integer for u8 {
    fn to_bytes(&self) -> Vec<u8> {
        self.to_be_bytes().to_vec()
    }

    fn from_bytes(bytes: &[u8]) -> Self {
        u8::from_be_bytes(bytes.try_into().unwrap())
    }

    fn byte_len() -> usize {
        std::mem::size_of::<Self>()
    }
}

impl Integer for u16 {
    fn to_bytes(&self) -> Vec<u8> {
        self.to_be_bytes().to_vec()
    }

    fn from_bytes(bytes: &[u8]) -> Self {
        u16::from_be_bytes(bytes.try_into().unwrap())
    }

    fn byte_len() -> usize {
        std::mem::size_of::<Self>()
    }
}

impl Integer for u32 {
    fn to_bytes(&self) -> Vec<u8> {
        self.to_be_bytes().to_vec()
    }

    fn from_bytes(bytes: &[u8]) -> Self {
        u32::from_be_bytes(bytes.try_into().unwrap())
    }

    fn byte_len() -> usize {
        std::mem::size_of::<Self>()
    }
}

impl Integer for u64 {
    fn to_bytes(&self) -> Vec<u8> {
        self.to_be_bytes().to_vec()
    }

    fn from_bytes(bytes: &[u8]) -> Self {
        u64::from_be_bytes(bytes.try_into().unwrap())
    }

    fn byte_len() -> usize {
        std::mem::size_of::<Self>()
    }
}

impl Integer for u128 {
    fn to_bytes(&self) -> Vec<u8> {
        self.to_be_bytes().to_vec()
    }

    fn from_bytes(bytes: &[u8]) -> Self {
        u128::from_be_bytes(bytes.try_into().unwrap())
    }

    fn byte_len() -> usize {
        std::mem::size_of::<Self>()
    }
}
