/// A 128-bit random number that uniquely identifies a connection.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Hash, Ord, PartialOrd)]
#[repr(transparent)]
pub struct Session(u128);

impl Session {
    pub fn new() -> Self {
        let lower = getrandom::u64().unwrap() as u128;
        let upper = getrandom::u64().unwrap() as u128;
        Self(lower | (upper << 64))
    }

    pub fn to_le_bytes(self) -> [u8; 16] {
        u128::to_le_bytes(self.0)
    }

    pub fn from_le_bytes(bytes: [u8; 16]) -> Self {
        Self(u128::from_le_bytes(bytes))
    }
}
