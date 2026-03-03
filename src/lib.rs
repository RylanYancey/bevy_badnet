use std::{
    io,
    net::{SocketAddr, TcpListener, ToSocketAddrs, UdpSocket},
    sync::Arc,
    time::Duration,
};

use bevy::{platform::collections::HashMap, prelude::*};
use crossbeam_channel::{Receiver, Sender, unbounded};

extern crate self as bevy_badnet;

pub mod prelude {
    pub use crate::{
        BadnetPlugin, NetContext,
        messages::{Connected, Disconnected},
        transport::{Tcp, Transport, Udp},
    };
}

use crate::{
    messages::{Connected, DisconnectReason, Disconnected},
    netsync::Netsync,
    prelude::{Tcp, Udp},
    runtime::{Event as RtEvent, Signal},
    session::Session,
    transport::{Protocol, Transport},
};

pub mod channel;
pub mod messages;
pub mod netsync;
pub mod packet;
mod runtime;
pub mod session;
pub mod transport;

#[derive(Default)]
pub struct BadnetPlugin {
    /// The frequency that the TCP Runtime ticks.
    pub tcp_tickrate: f32,

    /// If None, the UDP Socket will be bound to 0.0.0.0:0
    pub udp_port: Option<u16>,

    /// If None, no TCP Listener will be bound.
    pub tcp_listener_port: Option<u16>,
}

impl Plugin for BadnetPlugin {
    #[rustfmt::skip]
    fn build(&self, app: &mut App) {
        app
            .insert_resource(NetContext::new(self))
            .add_message::<Connected>()
            .add_message::<Disconnected>()
            .configure_sets(Update, (
                NetsyncSets::Decode
                    .after(NetcodeSets::Recv),
                NetsyncSets::Encode
                    .before(NetcodeSets::Send),
                NetcodeSets::Recv,
                NetcodeSets::Send,
            ))
            .add_systems(Update, (
                (
                    recv_packets,
                    netsync::recv_remote_entity_update_packets
                        .after(recv_packets),
                ).in_set(NetcodeSets::Recv),
                (
                    netsync::send_netsync_component_updates
                        .before(flush_transports::<Udp>),
                    flush_transports::<Udp>,
                    flush_transports::<Tcp>,
                ).in_set(NetcodeSets::Send)
            ))
        ;
    }
}

pub trait BadnetAppExt {
    fn add_netsync<T: Netsync>(&mut self) -> &mut Self;
}

impl BadnetAppExt for App {
    fn add_netsync<T: Netsync>(&mut self) -> &mut Self {
        self.add_systems(
            Update,
            (
                netsync::encode_sync_components::<T>.in_set(NetsyncSets::Encode),
                netsync::decode_sync_components::<T>.in_set(NetsyncSets::Decode),
            ),
        )
    }
}

/// Sets related to entity state synchronization.
#[derive(SystemSet, Copy, Clone, Eq, PartialEq, Debug, Hash)]
pub enum NetsyncSets {
    /// Encode local entity components into their send buffer.
    Encode,

    /// Decode just-received component data.
    Decode,
}

/// Sets related to network/connection management.
#[derive(SystemSet, Copy, Clone, Eq, PartialEq, Debug, Hash)]
pub enum NetcodeSets {
    /// The set in which information is received from TCP/UDP, and
    /// any events are emitted, such as Disconnected or Connected.
    Recv,

    /// The set in which information is sent over TCP/UDP by
    /// flushing send buffers. For TCP, this means sending buffered
    /// packets to the TCP runtime. For UDP, this means flushing
    /// any data left in the encoder.
    Send,
}

/// Main type that manages connections for both the client and server.
#[derive(Resource)]
pub struct NetContext {
    rt_signal_tx: Sender<Signal>,
    rt_event_rx: Receiver<RtEvent>,
    transports: HashMap<Session, Entity>,
    udp_socket: Arc<UdpSocket>,
    dc_req: HashMap<Session, DisconnectReason>,
}

impl NetContext {
    pub fn new(config: &BadnetPlugin) -> Self {
        let tcp_tickrate = Duration::from_secs_f32(1.0 / config.tcp_tickrate);
        let (signal_tx, signal_rx) = unbounded::<Signal>();
        let (event_tx, event_rx) = unbounded::<RtEvent>();
        runtime::rt_main(tcp_tickrate, signal_rx, event_tx);

        let udp_socket = UdpSocket::bind(format!("0.0.0.0:{}", config.udp_port.unwrap_or(0)))
            .expect("Failed to bind UDP Socket");
        udp_socket
            .set_nonblocking(true)
            .expect("Failed to set UDP Socket to Nonblocking.");

        if let Some(port) = config.tcp_listener_port {
            let tcp_listener =
                TcpListener::bind(format!("0.0.0.0:{port}")).expect("Failed to bind TcpListener.");
            tcp_listener
                .set_nonblocking(true)
                .expect("Failed to set TCP Listener to Nonblocking.");
            signal_tx
                .try_send(Signal::BindListener(mio::net::TcpListener::from_std(
                    tcp_listener,
                )))
                .expect("TCP Runtime crashed.");
        }

        Self {
            rt_event_rx: event_rx,
            rt_signal_tx: signal_tx,
            transports: HashMap::default(),
            udp_socket: Arc::new(udp_socket),
            dc_req: HashMap::new(),
        }
    }

    /// Returns true if disconnection was already requested, or the session is already disconnected.
    pub fn disconnect(&mut self, session: Session, reason: DisconnectReason) -> bool {
        if self.transports.contains_key(&session) {
            self.dc_req.insert(session, reason).is_some()
        } else {
            false
        }
    }
}

fn flush_transports<P: Protocol>(mut q: Query<&mut Transport<P>>) {
    for mut transport in &mut q {
        transport.flush();
    }
}

fn recv_packets(
    mut ctx: ResMut<NetContext>,
    mut dc_evs: MessageWriter<Disconnected>,
    mut conn_evs: MessageWriter<Connected>,
    mut commands: Commands,
    mut tcp_transports: Query<&mut Transport<Tcp>>,
) {
    while let Ok(ev) = ctx.rt_event_rx.try_recv() {
        match ev {
            RtEvent::Connected(session, addr) => {
                let entity = commands
                    .spawn((
                        Transport::new(Tcp::new(ctx.rt_signal_tx.clone(), session)),
                        Transport::new(Udp::new(session, ctx.udp_socket.clone(), addr)),
                    ))
                    .id();
                ctx.transports.insert(session, entity);
                conn_evs.write(Connected { session, entity });
            }
            RtEvent::RecvPackets(mut packets, session) => {
                if let Some(entity) = ctx.transports.get(&session) {
                    if let Ok(mut transport) = tcp_transports.get_mut(*entity) {
                        for (channel, data) in packets.drain(..) {
                            transport.add_incoming(channel, data);
                        }
                    }
                }
            }
            RtEvent::Disconnected(session, reason) => {
                if let Some(entity) = ctx.transports.remove(&session) {
                    dc_evs.write(Disconnected {
                        reason,
                        session,
                        entity,
                    });

                    commands
                        .entity(entity)
                        .remove::<Transport<Tcp>>()
                        .remove::<Transport<Udp>>();
                }
            }
        }
    }

    for (session, reason) in std::mem::take(&mut ctx.dc_req) {
        if let Some(entity) = ctx.transports.remove(&session) {
            ctx.rt_signal_tx
                .try_send(Signal::Disconnect(session, reason.clone()))
                .expect("TCP Runtime Crashed");
            dc_evs.write(Disconnected {
                session,
                entity,
                reason,
            });
            commands
                .entity(entity)
                .remove::<Transport<Tcp>>()
                .remove::<Transport<Udp>>();
        }
    }
}
