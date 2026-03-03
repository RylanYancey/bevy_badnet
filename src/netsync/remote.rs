use bevy::{
    ecs::{change_detection::Tick, entity::EntityHashMap},
    platform::collections::HashMap,
    prelude::*,
};
use bytes::{Buf, Bytes};

use crate::{
    channel::ChannelId,
    netsync::{local::SyncEntityId, version::Version},
};

#[derive(Resource, Default)]
pub struct RemoteEntities {
    by_sync_id: HashMap<SyncEntityId, Entity>,
    by_entity_id: EntityHashMap<SyncEntityId>,
}

impl RemoteEntities {
    pub fn get(&self, sync_id: SyncEntityId) -> Option<Entity> {
        self.by_sync_id.get(&sync_id).copied()
    }

    pub fn add(&mut self, sync_id: SyncEntityId, entity: Entity) {
        self.by_sync_id.insert(sync_id, entity);
        self.by_entity_id.insert(entity, sync_id);
    }
}

/// An entity whose state is synchronized with an entity on some remote.
#[derive(Component)]
pub struct RemoteEntity {
    incoming: HashMap<ChannelId, Bytes>,
    last_update: Tick,
    sync_id: SyncEntityId,
    version: Version,
}

impl RemoteEntity {
    pub fn new(sync_id: SyncEntityId, curr_tick: Tick) -> Self {
        Self {
            incoming: HashMap::new(),
            version: Version::ZERO,
            last_update: curr_tick,
            sync_id,
        }
    }

    pub const fn id(&self) -> SyncEntityId {
        self.sync_id
    }

    pub const fn version(&self) -> Version {
        self.version
    }

    pub const fn last_update_tick(&self) -> Tick {
        self.last_update
    }

    pub fn get_update(&self, id: ChannelId) -> Option<&Bytes> {
        self.incoming.get(&id)
    }

    /// Receive incoming component updates.
    ///
    /// Assumes the `SyncEntityId` and `Version` have already been read
    /// from the buffer, and the `SyncEntityId` was equal to `self.id()`,
    /// and the `Version` was newer than `self.version()`.
    pub fn unpack_component_updates(&mut self, mut buf: Bytes, tick: Tick, new_version: Version) {
        self.version = new_version;
        self.last_update = tick;
        self.incoming.clear();

        while buf.len() >= 4 {
            let sync_component_id = buf.get_u64_le();
            let segment_length = buf.get_u16_le() as usize;
            if buf.len() >= segment_length {
                let data = buf.split_to(segment_length);
                let id = ChannelId(sync_component_id);
                self.incoming.insert(id, data);
            } else {
                break;
            }
        }
    }
}
