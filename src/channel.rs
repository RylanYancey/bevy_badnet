use std::{fmt::Debug, hash::Hash};

use bevy::prelude::*;

/// A unique identifier for a channel.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Hash, Ord, PartialOrd)]
#[repr(transparent)]
pub struct ChannelId(pub(crate) u64);

impl ChannelId {
    pub const EXIT_CODE: Self = Self(0x420_69_420_69_420_69);
    pub const SESSION_ASSIGNMENT: Self = Self(0x69_420_69_420_69_420);

    pub fn new<T: TypePath>() -> Self {
        Self::from_str(T::type_path())
    }

    pub const fn from_str(s: &str) -> Self {
        Self(xxhash_rust::const_xxh64::xxh64(
            s.as_bytes(),
            0x9e3779b97f4a7c15,
        ))
    }
}

/// The channel on which sync entity updates are sent.
#[derive(TypePath)]
#[type_path(bevy_badnet)]
#[type_name(NetsyncEntityUpdates)]
pub struct NetsyncEntityUpdates;
