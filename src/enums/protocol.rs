/// The Engine.io protocol version used by the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum ProtocolVersion {
    V3 = 3,
}

impl Default for ProtocolVersion {
    fn default() -> Self {
        ProtocolVersion::V3
    }
}

impl From<u8> for ProtocolVersion {
    fn from(value: u8) -> Self {
        match value {
            3 => ProtocolVersion::V3,
            _ => ProtocolVersion::default(),
        }
    }
}

impl<'a> From<&'a u8> for ProtocolVersion {
    fn from(value: &'a u8) -> Self {
        match value {
            3 => ProtocolVersion::V3,
            _ => ProtocolVersion::default(),
        }
    }
}

impl From<ProtocolVersion> for u8 {
    fn from(value: ProtocolVersion) -> Self {
        value as u8
    }
}

impl From<ProtocolVersion> for String {
    fn from(value: ProtocolVersion) -> Self {
        (value as u8).to_string()
    }
}

impl ToString for ProtocolVersion {
    fn to_string(&self) -> String {
        match self {
            ProtocolVersion::V3 => "3".to_string(),
        }
    }
}
