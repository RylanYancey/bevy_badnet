use std::{
    io::{self, Read, Write},
    net::SocketAddr,
    time::Duration,
};

use bevy::platform::{collections::HashMap, time::Instant};
use mio::{Interest, Token, net::TcpStream};

use bytes::{Buf, BufMut, Bytes, BytesMut};
use crossbeam_channel::{Receiver, Sender, TryRecvError};

use crate::{channel::ChannelId, messages::DisconnectReason, session::Session};

/// Sent from Main Thread to Tcp Runtime
pub(crate) enum Signal {
    BindListener(mio::net::TcpListener),
    SendPackets(Vec<(ChannelId, Bytes)>, Session),
    Disconnect(Session, DisconnectReason),
}

/// Sent from TcpRuntime to Main Thread
pub(crate) enum Event {
    Connected(Session, SocketAddr),
    Disconnected(Session, DisconnectReason),
    RecvPackets(Vec<(ChannelId, Bytes)>, Session),
}

pub(crate) fn rt_main(tickrate: Duration, signal_rx: Receiver<Signal>, event_tx: Sender<Event>) {
    let mut by_session: HashMap<Session, Token> = HashMap::new();
    let mut pollables: Vec<Option<Pollable>> = Vec::new();
    let mut poll = mio::Poll::new().unwrap();
    let mut events = mio::Events::with_capacity(256);
    let mut disconnecting: Vec<TcpClient> = Vec::new();

    loop {
        loop {
            match signal_rx.try_recv() {
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
                Ok(Signal::BindListener(mut listener)) => {
                    let token = alloc_token(&mut pollables);
                    poll.registry()
                        .register(&mut listener, token, Interest::READABLE)
                        .unwrap();
                    pollables[token.0] = Some(Pollable::Listener(TcpListener { inner: listener }));
                }
                Ok(Signal::SendPackets(packets, session)) => {
                    if let Some(token) = by_session.get(&session) {
                        if let Some(Some(Pollable::Client(client))) = pollables.get_mut(token.0) {
                            for (channel, data) in packets {
                                client.encoder.encode(channel, &data);
                            }
                        }
                    }
                }
                Ok(Signal::Disconnect(session, reason)) => {
                    if let Some(token) = by_session.remove(&session) {
                        if let Some(pollable) = pollables.get_mut(token.0) {
                            disconnect_client(
                                &mut poll,
                                pollable,
                                &mut by_session,
                                &mut disconnecting,
                                &reason,
                            );
                        }
                    }
                }
            }
        }

        poll.poll(&mut events, Some(tickrate)).unwrap();
        for event in &events {
            if event.is_readable() {
                if let Some(pollable) = pollables.get_mut(event.token().0) {
                    match pollable {
                        Some(Pollable::Listener(listener)) => {
                            if let Ok((mut stream, addr)) = listener.inner.accept() {
                                let session = Session::new();
                                let token = alloc_token(&mut pollables);
                                // Register the stream with mio
                                poll.registry()
                                    .register(
                                        &mut stream,
                                        token,
                                        Interest::READABLE | Interest::WRITABLE,
                                    )
                                    .unwrap();
                                // construct new client
                                let mut client = TcpClient {
                                    stream,
                                    encoder: TcpEncoder {
                                        buffer: BytesMut::with_capacity(2048),
                                    },
                                    decoder: TcpDecoder {
                                        buffer: BytesMut::with_capacity(2048),
                                        header: None,
                                        limit: 1_000_000,
                                    },
                                    session,
                                    writable: true,
                                    last_flush: Instant::now(),
                                };
                                // send client their assigned session.
                                client
                                    .encoder
                                    .encode(ChannelId::SESSION_ASSIGNMENT, &session.to_le_bytes());
                                // add the client to the pollables vec
                                pollables[token.0] = Some(Pollable::Client(client));
                                // add the session to the resolver
                                by_session.insert(session, token);
                                // inform the main thread of the new connection.
                                if let Err(_) = event_tx.send(Event::Connected(session, addr)) {
                                    return;
                                }
                            }
                        }
                        Some(Pollable::Client(client)) => {
                            let (packets, reason) = client.read();
                            if packets.len() > 0 {
                                if event_tx
                                    .send(Event::RecvPackets(packets, client.session))
                                    .is_err()
                                {
                                    return;
                                }
                            }
                            if let Some(reason) = reason {
                                if event_tx
                                    .send(Event::Disconnected(client.session, reason.clone()))
                                    .is_err()
                                {
                                    return;
                                }
                                disconnect_client(
                                    &mut poll,
                                    pollable,
                                    &mut by_session,
                                    &mut disconnecting,
                                    &reason,
                                );
                            }
                        }
                        _ => {}
                    }
                }
            }

            if event.is_writable() {
                if let Some(pollable) = pollables.get_mut(event.token().0) {
                    if let Some(Pollable::Client(client)) = pollable {
                        client.writable = true;
                    }
                }
            }

            if event.is_write_closed() {
                if let Some(pollable) = pollables.get_mut(event.token().0) {
                    if let Some(Pollable::Client(client)) = pollable {
                        client.writable = false;
                    }
                }
            }
        }

        // Flush tcp encode buffers
        for pollable in &mut pollables {
            if let Some(Pollable::Client(client)) = pollable {
                if client.should_flush() {
                    if let Err(reason) = client.flush() {
                        if event_tx
                            .send(Event::Disconnected(client.session, reason.clone()))
                            .is_err()
                        {
                            return;
                        }
                        disconnect_client(
                            &mut poll,
                            pollable,
                            &mut by_session,
                            &mut disconnecting,
                            &reason,
                        );
                    }
                }
            }
        }

        // Attempt to finish writing messages to disconnecting clients.
        let mut i = 0;
        while i < disconnecting.len() {
            if disconnecting[i].flush().is_err() || disconnecting[i].encoder.buffer.is_empty() {
                disconnecting.swap_remove(i);
            } else {
                i += 1;
            }
        }
    }
}

fn alloc_token(pollables: &mut Vec<Option<Pollable>>) -> Token {
    Token(
        pollables
            .iter()
            .position(|v| v.is_none())
            .unwrap_or_else(|| {
                let i = pollables.len();
                pollables.push(None);
                i
            }),
    )
}

fn disconnect_client(
    poll: &mut mio::Poll,
    pollable: &mut Option<Pollable>,
    by_session: &mut HashMap<Session, Token>,
    disconnecting: &mut Vec<TcpClient>,
    reason: &DisconnectReason,
) {
    match std::mem::replace(pollable, None) {
        Some(Pollable::Client(mut client)) => {
            poll.registry().deregister(&mut client.stream).unwrap();
            let data = serde_json::to_string(&reason).unwrap();
            client.encoder.encode(ChannelId::EXIT_CODE, data.as_bytes());
            by_session.remove(&client.session);
            disconnecting.push(client);
        }
        _ => unreachable!(
            "disconnect_client must be called with a reference to a `Some(Pollable::Client())`"
        ),
    }
}

enum Pollable {
    Listener(TcpListener),
    Client(TcpClient),
}

struct TcpListener {
    inner: mio::net::TcpListener,
}

struct TcpClient {
    stream: TcpStream,
    encoder: TcpEncoder,
    decoder: TcpDecoder,
    session: Session,
    writable: bool,
    last_flush: Instant,
}

impl TcpClient {
    fn read(&mut self) -> (Vec<(ChannelId, Bytes)>, Option<DisconnectReason>) {
        let mut packets = Vec::new();
        loop {
            match self.decoder.read(&mut self.stream) {
                Err(reason) => return (packets, Some(reason)),
                Ok(0) => return (packets, None),
                Ok(_) => loop {
                    match self.decoder.decode() {
                        Ok(None) => break,
                        Ok(Some(packet)) => packets.push(packet),
                        Err(reason) => return (packets, Some(reason)),
                    }
                },
            }
        }
    }

    fn flush(&mut self) -> Result<(), DisconnectReason> {
        self.last_flush = Instant::now();
        self.encoder.flush(&mut self.stream)
    }

    /// A Tcp Encoder should flush if:
    /// - It is writable
    /// - Its buffer is not empty
    /// - 250 milliseconds have passed since last flush OR number of bytes is greater than 1024.
    fn should_flush(&self) -> bool {
        (self.writable && !self.encoder.buffer.is_empty())
            && (self.last_flush.elapsed() > Duration::from_millis(250)
                || self.encoder.buffer.len() > 1024)
    }
}

struct TcpDecoder {
    buffer: BytesMut,
    header: Option<(usize, ChannelId)>,
    limit: usize,
}

impl TcpDecoder {
    fn read(&mut self, stream: &mut TcpStream) -> Result<usize, DisconnectReason> {
        self.buffer.reserve(2048);
        let spare = unsafe {
            let spare = self.buffer.spare_capacity_mut();
            std::slice::from_raw_parts_mut(spare.as_mut_ptr().cast::<u8>(), spare.len())
        };

        loop {
            match stream.read(spare) {
                Ok(0) => return Err(DisconnectReason::Quit),
                Ok(amt) => {
                    unsafe { self.buffer.set_len(self.buffer.len() + amt) }
                    return Ok(amt);
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(0),
                Err(e) => {
                    return Err(DisconnectReason::from(e.kind()));
                }
            }
        }
    }

    fn decode(&mut self) -> Result<Option<(ChannelId, Bytes)>, DisconnectReason> {
        let (len, channel) = match self.header {
            Some(hdr) => hdr,
            None => {
                // Check if there are enough bytes in the buffer to read headers.
                if self.buffer.len() < 6 {
                    return Ok(None);
                }

                // read length header and channel.
                let channel = ChannelId(self.buffer.get_u64_le());
                let len = self.buffer.get_u64_le() as usize;

                // disconnect if the size of the packet is too large.
                if len > self.limit {
                    return Err(DisconnectReason::ProtocolViolation {
                        explanation: "Packet size limit exceeded.".into(),
                    });
                }

                self.header = Some((len, channel));
                (len, channel)
            }
        };

        if self.buffer.len() < len {
            self.buffer.reserve((len - self.buffer.len()).max(2048));
            Ok(None)
        } else {
            self.header = None;
            Ok(Some((channel, self.buffer.split_to(len).freeze())))
        }
    }
}

struct TcpEncoder {
    buffer: BytesMut,
}

impl TcpEncoder {
    fn encode(&mut self, channel: ChannelId, data: &[u8]) {
        self.buffer.put_u64_le(channel.0);
        self.buffer.put_u64_le(data.len() as u64);
        self.buffer.put_slice(data);
    }

    fn flush(&mut self, stream: &mut TcpStream) -> Result<(), DisconnectReason> {
        while self.buffer.has_remaining() {
            match stream.write(&self.buffer[..]) {
                Ok(0) => return Err(DisconnectReason::Quit),
                Ok(cnt) => self.buffer.advance(cnt),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(DisconnectReason::from(e.kind())),
            }
        }
        Ok(())
    }
}
