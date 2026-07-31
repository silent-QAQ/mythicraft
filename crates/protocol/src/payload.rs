use std::fmt;

use crate::{
    decode_varint, encode_varint, ProtocolError, MAX_CHANNEL_ID_BYTES, MAX_CUSTOM_PAYLOAD_BYTES,
};

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ChannelId(String);

impl ChannelId {
    pub fn parse(value: impl Into<String>) -> Result<Self, ProtocolError> {
        let value = value.into();
        if value.is_empty() || value.len() > MAX_CHANNEL_ID_BYTES {
            return Err(ProtocolError::InvalidChannelLength(
                i32::try_from(value.len()).unwrap_or(i32::MAX),
            ));
        }
        let Some((namespace, path)) = value.split_once(':') else {
            return Err(ProtocolError::InvalidChannelId(value));
        };
        if namespace.is_empty()
            || path.is_empty()
            || !namespace.bytes().all(is_namespace_byte)
            || !path.bytes().all(is_path_byte)
        {
            return Err(ProtocolError::InvalidChannelId(value));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ChannelId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CustomPayload {
    pub channel: ChannelId,
    pub schema_version: u16,
    pub payload: Vec<u8>,
}

pub fn encode_custom_payload(value: &CustomPayload) -> Result<Vec<u8>, ProtocolError> {
    validate_schema(value.schema_version.into())?;
    if value.payload.len() > MAX_CUSTOM_PAYLOAD_BYTES {
        return Err(ProtocolError::CustomPayloadTooLarge {
            actual: value.payload.len(),
        });
    }

    let mut encoded = Vec::with_capacity(
        value
            .channel
            .as_str()
            .len()
            .saturating_add(value.payload.len())
            .saturating_add(15),
    );
    encode_varint(value.channel.as_str().len() as i32, &mut encoded);
    encoded.extend_from_slice(value.channel.as_str().as_bytes());
    encode_varint(i32::from(value.schema_version), &mut encoded);
    encode_varint(value.payload.len() as i32, &mut encoded);
    encoded.extend_from_slice(&value.payload);
    Ok(encoded)
}

pub fn decode_custom_payload(input: &[u8]) -> Result<CustomPayload, ProtocolError> {
    let channel_length = decode_varint(input)?;
    if channel_length.value <= 0 || channel_length.value as usize > MAX_CHANNEL_ID_BYTES {
        return Err(ProtocolError::InvalidChannelLength(channel_length.value));
    }
    let channel_end = channel_length
        .consumed
        .checked_add(channel_length.value as usize)
        .ok_or(ProtocolError::TruncatedCustomPayload)?;
    let channel_bytes = input
        .get(channel_length.consumed..channel_end)
        .ok_or(ProtocolError::TruncatedCustomPayload)?;
    let channel_text = std::str::from_utf8(channel_bytes)
        .map_err(|_| ProtocolError::InvalidChannelUtf8)?
        .to_owned();
    let channel = ChannelId::parse(channel_text)?;

    let schema = decode_varint(
        input
            .get(channel_end..)
            .ok_or(ProtocolError::TruncatedCustomPayload)?,
    )?;
    validate_schema(schema.value)?;
    let schema_end = channel_end
        .checked_add(schema.consumed)
        .ok_or(ProtocolError::TruncatedCustomPayload)?;

    let payload_length = decode_varint(
        input
            .get(schema_end..)
            .ok_or(ProtocolError::TruncatedCustomPayload)?,
    )?;
    if payload_length.value < 0 {
        return Err(ProtocolError::NegativePayloadLength(payload_length.value));
    }
    let declared = payload_length.value as usize;
    if declared > MAX_CUSTOM_PAYLOAD_BYTES {
        return Err(ProtocolError::CustomPayloadTooLarge { actual: declared });
    }
    let payload_start = schema_end
        .checked_add(payload_length.consumed)
        .ok_or(ProtocolError::TruncatedCustomPayload)?;
    let actual = input.len().saturating_sub(payload_start);
    if actual != declared {
        return Err(ProtocolError::PayloadLengthMismatch { declared, actual });
    }

    Ok(CustomPayload {
        channel,
        schema_version: schema.value as u16,
        payload: input[payload_start..].to_vec(),
    })
}

fn validate_schema(value: i32) -> Result<(), ProtocolError> {
    if value <= 0 || value > i32::from(u16::MAX) {
        return Err(ProtocolError::InvalidSchemaVersion {
            actual: value,
            maximum: u16::MAX,
        });
    }
    Ok(())
}

const fn is_namespace_byte(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-' | b'.')
}

const fn is_path_byte(byte: u8) -> bool {
    is_namespace_byte(byte) || byte == b'/'
}

#[cfg(test)]
mod tests {
    use super::{decode_custom_payload, encode_custom_payload, ChannelId, CustomPayload};
    use crate::{encode_varint, ProtocolError, MAX_CUSTOM_PAYLOAD_BYTES};

    #[test]
    fn round_trips_window_three_namespace_payload() {
        let channel = ChannelId("mythicraft:message".to_owned());
        assert_eq!(ChannelId::parse(channel.as_str()), Ok(channel.clone()));
        let payload = CustomPayload {
            channel,
            schema_version: 1,
            payload: br#"{"namespace":"mythicraft"}"#.to_vec(),
        };
        let encoded = encode_custom_payload(&payload).unwrap_or_default();

        assert_eq!(decode_custom_payload(&encoded), Ok(payload));
    }

    #[test]
    fn rejects_uppercase_channel() {
        assert!(matches!(
            ChannelId::parse("Mythicraft:message"),
            Err(ProtocolError::InvalidChannelId(_))
        ));
    }

    #[test]
    fn rejects_declared_payload_length_mismatch() {
        let mut encoded = Vec::new();
        let channel = b"mythicraft:message";
        encode_varint(channel.len() as i32, &mut encoded);
        encoded.extend_from_slice(channel);
        encode_varint(1, &mut encoded);
        encode_varint(4, &mut encoded);
        encoded.extend_from_slice(&[1, 2, 3]);

        assert_eq!(
            decode_custom_payload(&encoded),
            Err(ProtocolError::PayloadLengthMismatch {
                declared: 4,
                actual: 3,
            })
        );
    }

    #[test]
    fn rejects_oversized_payload_before_reading_body() {
        let mut encoded = Vec::new();
        let channel = b"mythicraft:message";
        encode_varint(channel.len() as i32, &mut encoded);
        encoded.extend_from_slice(channel);
        encode_varint(1, &mut encoded);
        encode_varint((MAX_CUSTOM_PAYLOAD_BYTES + 1) as i32, &mut encoded);

        assert_eq!(
            decode_custom_payload(&encoded),
            Err(ProtocolError::CustomPayloadTooLarge {
                actual: MAX_CUSTOM_PAYLOAD_BYTES + 1,
            })
        );
    }
}
