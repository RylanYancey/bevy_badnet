use bevy::{ecs::entity::EntityHashSet, prelude::*};
use bytes::BufMut;

use crate::{channel::ChannelId, netsync::version::Version};

/// The SyncEntitys a Transport is subscribed to.
///
/// This Component should be attached to an entity with a `Transport<Udp>`.
#[derive(Component, Default, Clone, Eq, PartialEq)]
pub struct EntitySubscriptions(EntityHashSet);

impl EntitySubscriptions {
    pub fn iter(&self) -> impl Iterator<Item = &Entity> {
        self.0.iter()
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn add(&mut self, entity: Entity) -> bool {
        self.0.insert(entity)
    }

    pub fn remove(&mut self, entity: Entity) -> bool {
        self.0.remove(&entity)
    }
}

impl<'a> IntoIterator for &'a EntitySubscriptions {
    type IntoIter = bevy::ecs::entity::hash_set::Iter<'a>;
    type Item = &'a Entity;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

#[derive(Copy, Clone, Eq, PartialEq, Debug, Hash)]
pub struct SyncEntityId(u32);

impl SyncEntityId {
    pub fn new() -> Self {
        use bevy::platform::sync::atomic::{AtomicU32, Ordering};
        static NEXT_ID: AtomicU32 = AtomicU32::new(0);
        Self(NEXT_ID.fetch_add(1, Ordering::Relaxed))
    }

    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }
}

/// An entity whose state is sent to some remote.
#[derive(Component)]
pub struct LocalEntity {
    version: Version,
    sync_id: SyncEntityId,
    buffer: Vec<u8>,
}

impl LocalEntity {
    pub fn new() -> Self {
        let sync_id = SyncEntityId::new();
        let mut version = Version::ONE;
        let mut buffer = Vec::new();
        buffer.put_u32_le(sync_id.0);
        buffer.put_u32_le(version.take().0);
        Self {
            version,
            buffer,
            sync_id,
        }
    }

    /// Get the syncronize ID of the entity.
    pub const fn id(&self) -> SyncEntityId {
        self.sync_id
    }

    /// Write component data to the sync entitys send buffer.
    pub fn encode(&mut self, component_id: ChannelId, bytes: impl AsRef<[u8]>) {
        let bytes = bytes.as_ref();
        self.buffer.put_u16_le(component_id.0 as u16);
        self.buffer.put_u16_le(bytes.len() as u16);
        self.buffer.put_slice(bytes)
    }

    /// Clear the send buffer and increment the version.
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.buffer.put_u32_le(self.sync_id.0);
        self.buffer.put_u32_le(self.version.take().0);
    }

    pub fn get_send_buffer(&self) -> &[u8] {
        &self.buffer[..]
    }
}
