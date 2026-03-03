use bevy::{
    ecs::{component::Mutable, system::SystemChangeTick},
    prelude::*,
};
use bytes::Buf;

use crate::{
    channel::{ChannelId, NetsyncEntityUpdates},
    netsync::{
        codec::{Decoder, Encoder, TransformCodec},
        local::{EntitySubscriptions, LocalEntity, SyncEntityId},
        remote::{RemoteEntities, RemoteEntity},
        version::Version,
    },
    prelude::{Transport, Udp},
};

pub mod codec;
pub mod local;
pub mod remote;
pub mod version;

pub trait Netsync: Component<Mutability = Mutable> + TypePath + Sized {
    type Encoder: Encoder<Self>;
    type Decoder: Decoder<Self>;
}

pub(crate) fn encode_sync_components<N: Netsync>(
    mut encoder: Local<N::Encoder>,
    mut q: Query<(&mut LocalEntity, &N)>,
) {
    let id = ChannelId::new::<N>();

    for (mut entity, component) in &mut q {
        let bytes = encoder.encode(component);
        entity.encode(id, bytes);
    }
}

pub(crate) fn decode_sync_components<N: Netsync>(
    tick: SystemChangeTick,
    mut decoder: Local<N::Decoder>,
    mut q: Query<(Entity, &RemoteEntity, Option<&mut N>)>,
    mut commands: Commands,
) {
    let id = ChannelId::new::<N>();

    for (entity, remote, value) in &mut q {
        if remote
            .last_update_tick()
            .is_newer_than(tick.last_run(), tick.this_run())
        {
            if let Some(data) = remote.get_update(id) {
                match decoder.decode(&data) {
                    Err(e) => warn!(
                        "Failed to decode sync component '{}' with error: '{e:?}'",
                        N::type_path()
                    ),
                    Ok(item) => {
                        if let Some(mut value) = value {
                            *value = item;
                        } else {
                            commands.entity(entity).insert(item);
                        }
                    }
                }
            } else {
                if value.is_some() {
                    commands.entity(entity).remove::<N>();
                }
            }
        }
    }
}

pub(crate) fn recv_remote_entity_update_packets(
    tick: SystemChangeTick,
    mut q_transports: Query<&mut Transport<Udp>>,
    mut remote_entities: ResMut<RemoteEntities>,
    mut q_remote_entity: Query<&mut RemoteEntity>,
    mut commands: Commands,
) {
    let netsync_channel = ChannelId::new::<NetsyncEntityUpdates>();

    for mut transport in &mut q_transports {
        for mut packet in transport.recv(netsync_channel) {
            if packet.len() >= 8 {
                let sync_id = SyncEntityId::from_bits(packet.get_u32_le());
                let version = Version::from_bits(packet.get_u32_le());

                if let Some(entity) = remote_entities.get(sync_id) {
                    // update the existing remote entity.
                    if let Ok(mut remote) = q_remote_entity.get_mut(entity) {
                        if version.is_newer_than(remote.version()) {
                            remote.unpack_component_updates(packet, tick.this_run(), version);
                        }
                    }
                } else {
                    // Create a new remote entity.
                    let mut remote = RemoteEntity::new(sync_id, tick.this_run());
                    remote.unpack_component_updates(packet, tick.this_run(), version);
                    let entity_id = commands.spawn(remote).id();
                    remote_entities.add(sync_id, entity_id);
                }
            }
        }
    }
}

pub(crate) fn send_netsync_component_updates(
    mut q_transports: Query<(&mut Transport<Udp>, &EntitySubscriptions)>,
    mut q_sync_entity: Query<&mut LocalEntity>,
) {
    let netsync_channel_id = ChannelId::new::<NetsyncEntityUpdates>();

    for (mut transport, subscriptions) in &mut q_transports {
        for entity in subscriptions {
            if let Ok(entity) = q_sync_entity.get(*entity) {
                transport.send(netsync_channel_id, entity.get_send_buffer())
            }
        }
    }

    for mut entity in &mut q_sync_entity {
        entity.clear();
    }
}

impl Netsync for Transform {
    type Encoder = TransformCodec;
    type Decoder = TransformCodec;
}
