//! Wire protocol between the timeline UI and the renderer.
//!
//! Messages are JSON documents, one per UDP datagram, on localhost. The UI is
//! the only side that needs to send anything for the system to work; the
//! renderer replies to [`Message::Hello`] with a [`Message::Describe`] so a UI
//! can discover the parameters of whatever graph is currently loaded.

use std::io;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};

use serde::{Deserialize, Serialize};

use crate::graph::{GraphStructure, ParamPath};
use crate::params::{ParamDescriptor, ParamValue};

/// Bumped whenever a message shape changes incompatibly. Both sides announce
/// it; a mismatch is reported to the user instead of silently misbehaving.
pub const PROTOCOL_VERSION: u32 = 2;

/// Default UDP port the renderer listens on.
pub const DEFAULT_PORT: u16 = 9471;

/// Upper bound on a single datagram. Describe messages for large graphs are
/// chunked to stay below it.
pub const MAX_DATAGRAM: usize = 60 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "msg", rename_all = "snake_case")]
pub enum Message {
    /// UI → renderer: announce yourself and ask for a description.
    Hello { client: String, protocol: u32 },
    /// Renderer → UI: what the graph looks like, in UI terms only (no node
    /// kinds, shaders or engine types). Sent before `Describe`.
    Structure {
        protocol: u32,
        structure: GraphStructure,
    },
    /// Renderer → UI: the parameters of the loaded graph. May arrive in
    /// several chunks (`chunk` of `chunks`).
    Describe {
        protocol: u32,
        graph: String,
        chunk: u16,
        chunks: u16,
        params: Vec<ParamDescriptor>,
    },
    /// UI → renderer: set one parameter (an override that wins over the graph's base value).
    SetParam { path: ParamPath, value: ParamValue },
    /// UI → renderer: set several parameters at once.
    SetParams {
        values: Vec<(ParamPath, ParamValue)>,
    },
    /// UI → renderer: drop the override for one parameter, back to the graph's base value.
    ClearParam { path: ParamPath },
    /// UI → renderer: drop all overrides.
    ClearAll,
    /// UI → renderer: informational transport state (for on-screen display / debugging).
    Transport { time: f32, playing: bool },
    /// UI → renderer: liveness probe.
    Ping,
    /// Renderer → UI: reply to `Ping`.
    Pong { protocol: u32 },
}

impl Message {
    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("Message is always serializable")
    }

    pub fn decode(bytes: &[u8]) -> serde_json::Result<Self> {
        serde_json::from_slice(bytes)
    }

    /// Split a parameter description into datagram-sized chunks.
    pub fn describe(graph: &str, params: &[ParamDescriptor]) -> Vec<Message> {
        // Conservative: a descriptor with options and a doc string is a few hundred bytes.
        const PER_CHUNK: usize = 48;
        let chunks = params.chunks(PER_CHUNK).collect::<Vec<_>>();
        let total = chunks.len().max(1) as u16;
        if chunks.is_empty() {
            return vec![Message::Describe {
                protocol: PROTOCOL_VERSION,
                graph: graph.to_owned(),
                chunk: 0,
                chunks: 1,
                params: Vec::new(),
            }];
        }
        chunks
            .into_iter()
            .enumerate()
            .map(|(i, part)| Message::Describe {
                protocol: PROTOCOL_VERSION,
                graph: graph.to_owned(),
                chunk: i as u16,
                chunks: total,
                params: part.to_vec(),
            })
            .collect()
    }

    pub fn pong() -> Message {
        Message::Pong {
            protocol: PROTOCOL_VERSION,
        }
    }

    pub fn hello(client: &str) -> Message {
        Message::Hello {
            client: client.to_owned(),
            protocol: PROTOCOL_VERSION,
        }
    }

    pub fn structure(structure: GraphStructure) -> Message {
        Message::Structure {
            protocol: PROTOCOL_VERSION,
            structure,
        }
    }
}

/// Non-blocking UDP receiver used by the renderer.
pub struct ParamReceiver {
    socket: UdpSocket,
    buf: Vec<u8>,
}

impl ParamReceiver {
    pub fn bind(port: u16) -> io::Result<Self> {
        let socket = UdpSocket::bind(("127.0.0.1", port))?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            buf: vec![0; MAX_DATAGRAM + 1024],
        })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    /// Drain every pending datagram. Undecodable datagrams are skipped.
    pub fn poll(&mut self) -> Vec<(Message, SocketAddr)> {
        let mut out = Vec::new();
        loop {
            match self.socket.recv_from(&mut self.buf) {
                Ok((len, from)) => {
                    if let Ok(msg) = Message::decode(&self.buf[..len]) {
                        out.push((msg, from));
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
        out
    }

    /// Reply to a specific peer (used for `Describe`).
    pub fn send_to(&self, msg: &Message, to: SocketAddr) -> io::Result<()> {
        self.socket.send_to(&msg.encode(), to).map(|_| ())
    }
}

/// UDP sender used by the timeline UI. Also receives replies.
pub struct ParamSender {
    socket: UdpSocket,
    target: SocketAddr,
    buf: Vec<u8>,
}

impl ParamSender {
    pub fn connect(target: impl ToSocketAddrs) -> io::Result<Self> {
        let target = target
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no address"))?;
        let socket = UdpSocket::bind(("127.0.0.1", 0))?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            target,
            buf: vec![0; MAX_DATAGRAM + 1024],
        })
    }

    pub fn target(&self) -> SocketAddr {
        self.target
    }

    pub fn send(&self, msg: &Message) -> io::Result<()> {
        self.socket.send_to(&msg.encode(), self.target).map(|_| ())
    }

    /// Drain replies (e.g. `Describe`).
    pub fn poll(&mut self) -> Vec<Message> {
        let mut out = Vec::new();
        while let Ok((len, _)) = self.socket.recv_from(&mut self.buf) {
            if let Ok(msg) = Message::decode(&self.buf[..len]) {
                out.push(msg);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Graph;
    use crate::node_def::NodeLibrary;

    #[test]
    fn message_roundtrip() {
        let msg = Message::SetParam {
            path: ParamPath::new("warp", "amount"),
            value: ParamValue::Float(0.25),
        };
        let bytes = msg.encode();
        assert_eq!(
            std::str::from_utf8(&bytes).unwrap(),
            r#"{"msg":"set_param","path":"warp.amount","value":0.25}"#
        );
        assert_eq!(Message::decode(&bytes).unwrap(), msg);
    }

    #[test]
    fn structure_message_fits_a_datagram_and_carries_no_engine_types() {
        let msg = Message::structure(Graph::showcase().structure(&NodeLibrary::builtin()));
        let bytes = msg.encode();
        assert!(bytes.len() < MAX_DATAGRAM);
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(
            !text.contains("wgsl") && !text.contains("\"source\":"),
            "{text}"
        );
        assert_eq!(Message::decode(&bytes).unwrap(), msg);
    }

    #[test]
    fn typed_values_roundtrip_on_the_wire() {
        let msg = Message::SetParams {
            values: vec![
                (
                    ParamPath::new("blend", "mode"),
                    ParamValue::Choice("add".into()),
                ),
                (
                    ParamPath::new("solid", "color"),
                    ParamValue::Color([1.0, 0.0, 0.0, 1.0]),
                ),
                (ParamPath::new("x", "on"), ParamValue::Bool(true)),
            ],
        };
        assert_eq!(Message::decode(&msg.encode()).unwrap(), msg);
    }

    #[test]
    fn describe_chunks_cover_all_params() {
        let params = Graph::showcase().describe_params(&NodeLibrary::builtin());
        let msgs = Message::describe("showcase", &params);
        let mut total = 0;
        for m in &msgs {
            assert!(m.encode().len() < MAX_DATAGRAM);
            if let Message::Describe { params, .. } = m {
                total += params.len();
            }
        }
        assert_eq!(total, params.len());
    }

    #[test]
    fn udp_loopback() {
        let mut rx = ParamReceiver::bind(0).unwrap();
        let addr = rx.local_addr().unwrap();
        let mut tx = ParamSender::connect(addr).unwrap();
        tx.send(&Message::hello("test")).unwrap();
        let mut got = Vec::new();
        for _ in 0..50 {
            got = rx.poll();
            if !got.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(got.len(), 1);
        let (msg, from) = &got[0];
        assert!(matches!(msg, Message::Hello { .. }));
        rx.send_to(&Message::ClearAll, *from).unwrap();
        let mut reply = Vec::new();
        for _ in 0..50 {
            reply = tx.poll();
            if !reply.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(reply, vec![Message::ClearAll]);
    }
}
