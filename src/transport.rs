use std::{
    io,
    net::{SocketAddr, UdpSocket},
    sync::Arc,
    time::Duration,
};

use bevy::{
    ecs::component::Component,
    platform::{collections::HashMap, time::Instant},
};
use bytes::{BufMut, Bytes};
use crossbeam_channel::Sender;

use crate::{channel::ChannelId, packet::Payload, runtime::Signal, session::Session};

/// Inner<Udp> is pretty large, so this type is boxed to avoid copying all that around.
struct Inner<P> {
    protocol: P,
    incoming: HashMap<ChannelId, Vec<Bytes>>,
    connected_at: Instant,
    stats: TransportStats,
}

#[derive(Component)]
pub struct Transport<P>(Box<Inner<P>>);

impl<P> Transport<P>
where
    P: Protocol,
{
    pub(crate) fn new(protocol: P) -> Self {
        Self(Box::new(Inner {
            protocol,
            incoming: HashMap::new(),
            connected_at: Instant::now(),
            stats: TransportStats::default(),
        }))
    }

    pub(crate) fn add_incoming(&mut self, id: ChannelId, data: Bytes) {
        if let Some(buf) = self.0.incoming.get_mut(&id) {
            buf.push(data);
        } else {
            self.0.incoming.insert(id, vec![data]);
        }
    }

    #[inline]
    pub fn session(&self) -> Session {
        self.0.protocol.session()
    }

    pub const fn stats(&self) -> &TransportStats {
        &self.0.stats
    }

    /// Send a message on this transport.
    pub fn send(&mut self, on: ChannelId, payload: impl Payload) {
        self.0.stats.add_sent(payload.as_bytes().len());
        self.0.protocol.send(on, payload);
    }

    /// Receive packets for this connection on the given channel.
    pub fn recv(&mut self, on: ChannelId) -> impl Iterator<Item = Bytes> {
        self.0
            .incoming
            .get_mut(&on)
            .map(|inner| inner.drain(..))
            .into_iter()
            .flatten()
    }

    pub(crate) fn flush(&mut self) {
        self.0.protocol.flush();
    }

    pub fn uptime(&self) -> Duration {
        self.0.connected_at.elapsed()
    }
}

/// Statistical information about a transport.
#[derive(Clone, Debug, Default)]
pub struct TransportStats {
    /// Number of packets sent since the transport was created.
    pub packets_sent_total: u32,

    /// Number of bytes sent since the transport was created.
    pub bytes_sent_total: usize,

    /// Number of packets sent this tick.
    pub packets_sent_recent: u32,

    /// Number of packets sent this tick.
    pub bytes_sent_recent: usize,

    /// Number of packets received since the transport was created.
    pub packets_recv_total: u32,

    /// Number of bytes received since the transport was created.
    pub bytes_recv_total: usize,

    /// Number of packets received this tick.
    pub packets_recv_recent: u32,

    /// Number of bytes received this tick.
    pub bytes_recv_recent: usize,
}

impl TransportStats {
    pub fn add_sent(&mut self, byte_count: usize) {
        self.packets_sent_recent += 1;
        self.bytes_sent_recent += byte_count;
    }

    pub fn add_recv(&mut self, byte_count: usize) {
        self.packets_recv_recent += 1;
        self.bytes_recv_recent += byte_count;
    }

    pub fn update_total(&mut self) {
        self.packets_sent_total += self.packets_sent_recent;
        self.packets_sent_recent = 0;
        self.bytes_sent_total += self.bytes_sent_recent;
        self.bytes_sent_recent = 0;
        self.packets_recv_total += self.packets_recv_recent;
        self.packets_recv_total = 0;
        self.bytes_recv_total += self.bytes_recv_recent;
        self.bytes_recv_recent = 0;
    }
}

pub trait Protocol: Send + Sync + 'static {
    fn send(&mut self, on: ChannelId, payload: impl Payload);
    fn session(&self) -> Session;
    fn flush(&mut self);
}

pub struct Tcp {
    session: Session,
    packets: Vec<(ChannelId, Bytes)>,
    tx: Sender<Signal>,
}

impl Tcp {
    pub(crate) fn new(tx: Sender<Signal>, session: Session) -> Self {
        Self {
            session,
            tx,
            packets: Vec::new(),
        }
    }
}

impl Protocol for Tcp {
    fn send(&mut self, on: ChannelId, payload: impl Payload) {
        self.packets.push((on, payload.into_bytes()))
    }

    fn session(&self) -> Session {
        self.session
    }

    fn flush(&mut self) {
        if self.packets.len() > 0 {
            let _ = self.tx.send(Signal::SendPackets(
                std::mem::take(&mut self.packets),
                self.session,
            ));
        }
    }
}

pub struct Udp {
    session: Session,
    /// Buffer for combining updates into a single udp write.
    buffer: Vec<u8>,
    /// A UDP Socket set to Nonblocking.
    socket: Arc<UdpSocket>,
    /// The address to send data to.
    target: SocketAddr,
}

impl Udp {
    pub(crate) fn new(session: Session, socket: Arc<UdpSocket>, addr: SocketAddr) -> Self {
        let mut buffer = Vec::new();
        buffer.extend_from_slice(&session.to_le_bytes());
        Self {
            session,
            buffer: Vec::new(),
            socket,
            target: addr,
        }
    }
}

impl Protocol for Udp {
    fn session(&self) -> Session {
        self.session
    }

    fn send(&mut self, on: ChannelId, payload: impl Payload) {
        let bytes = payload.as_bytes();
        if bytes.len() + 4 + self.buffer.len() >= 1200 {
            self.flush();
        }
        self.buffer.put_u64_le(on.0);
        self.buffer.put_u16_le(bytes.len() as u16);
        self.buffer.put_slice(bytes);
    }

    fn flush(&mut self) {
        if self.buffer.len() > 16 {
            let mut i = 0;
            // UDP sends can be interrupted at random or can block if overloaded, so I retry 3 times.
            // Unlike TCP, we care about latency more than reliability, so we don't mind losing packets.
            loop {
                match self.socket.send_to(&self.buffer, self.target) {
                    Err(e)
                        if e.kind() == io::ErrorKind::Interrupted
                            || e.kind() == io::ErrorKind::WouldBlock && i < 3 =>
                    {
                        i += 1;
                        continue;
                    }
                    _ => break,
                }
            }

            // clear the buffer and re-write the session.
            self.buffer.clear();
            self.buffer.extend_from_slice(&self.session.to_le_bytes());
        }
    }
}
