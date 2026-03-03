#[derive(Copy, Clone, Eq, PartialEq, Debug, Hash, Default, Ord, PartialOrd)]
pub struct Version(pub(crate) u32);

impl Version {
    pub const ZERO: Self = Self(0);
    pub const ONE: Self = Self(1);

    pub const fn take(&mut self) -> Version {
        let ret = *self;
        self.0 += 1;
        ret
    }

    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    pub const fn to_bits(self) -> u32 {
        self.0
    }

    pub const fn is_newer_than(self, other: Self) -> bool {
        self.0 > other.0
    }
}
