use serde::{Deserialize, Serialize};
use serde_json;
use std::error::Error;
use std::str;

use crate::enums::packet::PacketType;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct Packet {
    #[serde(rename = "type")]
    pub packet_type: PacketType,
    pub nsp: Option<String>,
    pub target: Option<String>,
    pub data: Option<serde_json::Value>,
}

impl Packet {
    pub fn new(
        packet_type: PacketType,
        nsp: Option<String>,
        target: Option<String>,
        data: Option<serde_json::Value>,
    ) -> Self {
        Packet {
            packet_type,
            nsp,
            target,
            data,
        }
    }

    pub fn new_raw(
        packet_type: PacketType,
        nsp: Option<String>,
        target: Option<String>,
        data: String,
    ) -> Self {
        Packet {
            packet_type,
            nsp,
            target,
            data: Some(serde_json::from_str::<serde_json::Value>(&data).unwrap_or_default()),
        }
    }

    pub fn data_is_valid(&self) -> bool {
        if self.data.is_none() {
            return false;
        }

        let data = self.data.as_ref().unwrap();

        match self.packet_type {
            PacketType::Connect => data.is_object() || data.is_null(),
            PacketType::Disconnect => data.is_null(),
            PacketType::Event => data.is_array() && !data.as_array().unwrap().is_empty(),
            PacketType::Ack => data.is_array(),
            PacketType::ConnectError => data.is_object(),
            _ => false,
        }
    }

    pub fn decode(chunk: impl AsRef<[u8]>) -> Result<Packet, Box<dyn Error>> {
        let chunk_str = str::from_utf8(&chunk.as_ref())?;
        let chunk_len = chunk_str.len();

        if chunk_len < 4 {
            let packet_type: PacketType = chunk_str
                .chars()
                .filter(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse::<u8>()?
                .into();
            return Ok(Packet::new(packet_type, None, None, None));
        }

        if (chunk_str.starts_with("0{") || chunk_str.starts_with("0\\{")) && chunk_len > 5 {
            return Ok(Packet::new(
                PacketType::Connect,
                None,
                None,
                serde_json::from_str::<serde_json::Value>(&chunk_str[1..]).ok(),
            ));
        }

        let (chunk_identifier, payload) = chunk_str.split_once(',').unwrap_or(("", ""));

        let (chunk_identifier, namespace) = chunk_identifier
            .split_once('/')
            .unwrap_or((chunk_identifier, ""));

        let namespace = match namespace.is_empty() {
            true => None,
            false => Some(namespace.to_owned()),
        };

        let packet_type: PacketType = chunk_identifier
            .chars()
            .take(3)
            .filter(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse::<u8>()?
            .into();

        if chunk_len < 5 {
            return Ok(Packet::new(packet_type, namespace, None, None));
        }

        let mut parsed_payload = None;
        let mut target = None;

        if !payload.is_empty() {
            if payload.starts_with('[') {
                let (target_str, payload) = payload[1..payload.len() - 1]
                    .split_once(',')
                    .unwrap_or(("", ""));

                if !target_str.is_empty() {
                    target = Some(target_str[1..target_str.len() - 1].to_owned());
                }

                parsed_payload = serde_json::from_str::<serde_json::Value>(payload).ok();
            } else {
                parsed_payload = serde_json::from_str::<serde_json::Value>(payload).ok();
            }
        }

        Ok(Packet::new(packet_type, namespace, target, parsed_payload))
    }

    pub fn decode_raw<'p>(
        chunk: impl AsRef<&'p [u8]>,
    ) -> Result<(Packet, Option<&'p str>), Box<dyn Error>> {
        let chunk_str = str::from_utf8(&chunk.as_ref())?;
        let chunk_len = chunk_str.len();

        if chunk_len < 4 {
            let packet_type: PacketType = chunk_str
                .chars()
                .filter(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse::<u8>()?
                .into();
            return Ok((Packet::new(packet_type, None, None, None), None));
        }

        if chunk_str.starts_with('0') && chunk_len > 5 {
            return Ok((
                Packet::new(
                    PacketType::Connect,
                    None,
                    None,
                    serde_json::from_str::<serde_json::Value>(&chunk_str[1..]).ok(),
                ),
                None,
            ));
        }

        let (chunk_identifier, payload) = chunk_str.split_once(',').unwrap_or(("", ""));

        let (chunk_identifier, target) = chunk_identifier
            .split_once('/')
            .unwrap_or((chunk_identifier, ""));

        let target = match target.is_empty() {
            true => None,
            false => Some(target.to_owned()),
        };

        let packet_type: PacketType = chunk_identifier
            .chars()
            .take(3)
            .filter(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse::<u8>()?
            .into();

        if chunk_len < 5 {
            return Ok((Packet::new(packet_type, None, target, None), Some(payload)));
        }

        Ok((Packet::new(packet_type, None, target, None), Some(payload)))
    }

    pub fn encode(packet: Packet) -> String {
        let mut namespace = String::new();

        if packet.nsp.is_none() && packet.target.is_none() && packet.data.is_none() {
            return format!("{packet_type}", packet_type = packet.packet_type as u8);
        }

        if let Some(nsp) = packet.nsp {
            namespace = format!("/{}", nsp);
        }

        if packet.data.is_none() {
            return format!(
                "{packet_type}{namespace},",
                packet_type = packet.packet_type as u8
            );
        }

        let data = packet.data.unwrap().to_string();

        match packet.packet_type {
            PacketType::Connect => {
                format!(
                    "{packet_type}{namespace}{data}",
                    packet_type = packet.packet_type as u8
                )
            }
            PacketType::Message | PacketType::Ping => {
                if let Some(target) = packet.target {
                    format!(
                        "{packet_type}{namespace},[\"{target}\",{data}]",
                        packet_type = packet.packet_type as u8,
                    )
                } else {
                    format!(
                        "{packet_type}{namespace},[{data}]",
                        packet_type = packet.packet_type as u8,
                    )
                }
            }
            _ => format!(
                "{packet_type}{namespace}[{data}]",
                packet_type = packet.packet_type as u8
            ),
        }
    }
}
