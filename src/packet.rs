use bytes::Bytes;

pub trait Payload {
    fn as_bytes(&self) -> &[u8];
    fn into_bytes(self) -> Bytes;
}

impl Payload for Vec<u8> {
    fn as_bytes(&self) -> &[u8] {
        self.as_slice()
    }

    fn into_bytes(self) -> Bytes {
        Bytes::from(self)
    }
}

impl Payload for &[u8] {
    fn as_bytes(&self) -> &[u8] {
        self
    }

    fn into_bytes(self) -> Bytes {
        Bytes::copy_from_slice(self)
    }
}

impl Payload for Bytes {
    fn as_bytes(&self) -> &[u8] {
        &self[..]
    }

    fn into_bytes(self) -> Bytes {
        self
    }
}
