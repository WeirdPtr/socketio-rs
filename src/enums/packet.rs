use serde::{Deserialize, Serialize};

/// Packet type enum
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(u8)]
pub enum PacketType {
    Connect = 0,
    Disconnect = 1,
    Event = 2,
    Ack = 3,
    ConnectError = 4,
    BinaryEvent = 5,
    BinaryAck = 6,
    Ping = 40,
    Message = 42,
}

impl Default for PacketType {
    fn default() -> Self {
        PacketType::Message
    }
}

impl From<u8> for PacketType {
    fn from(value: u8) -> Self {
        match value {
            0 => PacketType::Connect,
            1 => PacketType::Disconnect,
            2 => PacketType::Event,
            3 => PacketType::Ack,
            4 => PacketType::ConnectError,
            5 => PacketType::BinaryEvent,
            6 => PacketType::BinaryAck,
            40 => PacketType::Ping,
            42 => PacketType::Message,
            _ => PacketType::default(),
        }
    }
}

impl From<PacketType> for u8 {
    fn from(value: PacketType) -> Self {
        value as u8
    }
}

impl From<PacketType> for String {
    fn from(value: PacketType) -> Self {
        (value as u8).to_string()
    }
}

impl From<PacketType> for &'static str {
    fn from(value: PacketType) -> Self {
        match value {
            PacketType::Connect => "connect",
            PacketType::Disconnect => "disconnect",
            PacketType::Event => "event",
            PacketType::Ack => "ack",
            PacketType::ConnectError => "connect_error",
            PacketType::BinaryEvent => "binary_event",
            PacketType::BinaryAck => "binary_ack",
            PacketType::Ping => "ping",
            PacketType::Message => "message",
        }
    }
}

impl<'a> From<&'a str> for PacketType {
    fn from(value: &'a str) -> Self {
        match value {
            "connect" => PacketType::Connect,
            "disconnect" => PacketType::Disconnect,
            "event" => PacketType::Event,
            "ack" => PacketType::Ack,
            "connect_error" => PacketType::ConnectError,
            "binary_event" => PacketType::BinaryEvent,
            "binary_ack" => PacketType::BinaryAck,
            "ping" => PacketType::Ping,
            "message" => PacketType::Message,
            _ => PacketType::default(),
        }
    }
}
