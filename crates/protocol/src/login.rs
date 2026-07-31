use crate::io::{write_bool, write_byte_array, write_string, write_uuid, PacketCursor};
use crate::{encode_varint, PacketFrame, ProtocolError};
use uuid::Uuid;

pub const LOGIN_START_PACKET_ID: i32 = 0;
pub const LOGIN_ENCRYPTION_RESPONSE_PACKET_ID: i32 = 1;
pub const LOGIN_ACKNOWLEDGED_PACKET_ID: i32 = 3;
pub const LOGIN_DISCONNECT_PACKET_ID: i32 = 0;
pub const LOGIN_ENCRYPTION_REQUEST_PACKET_ID: i32 = 1;
pub const LOGIN_FINISHED_PACKET_ID: i32 = 2;
pub const LOGIN_COMPRESSION_PACKET_ID: i32 = 3;

const MAX_USERNAME_BYTES: usize = 16;
const MAX_SERVER_ID_BYTES: usize = 20;
const MAX_PUBLIC_KEY_BYTES: usize = 8 * 1024;
const MAX_VERIFY_TOKEN_BYTES: usize = 512;
const MAX_ENCRYPTED_SECRET_BYTES: usize = 512;
const MAX_LOGIN_JSON_BYTES: usize = 32_767;
const MAX_PROFILE_PROPERTIES: usize = 64;
const MAX_PROPERTY_NAME_BYTES: usize = 16;
const MAX_PROPERTY_VALUE_BYTES: usize = 32_767;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoginStart {
    pub name: String,
    pub profile_id: Uuid,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncryptionResponse {
    pub shared_secret: Vec<u8>,
    pub verify_token: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LoginAcknowledged;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoginDisconnect {
    pub json_reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncryptionRequest {
    pub server_id: String,
    pub public_key: Vec<u8>,
    pub verify_token: Vec<u8>,
    pub should_authenticate: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SetCompression {
    pub threshold: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProfileProperty {
    pub name: String,
    pub value: String,
    pub signature: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoginFinished {
    pub profile_id: Uuid,
    pub name: String,
    pub properties: Vec<ProfileProperty>,
    pub session_id: Uuid,
}

pub fn decode_login_start(frame: &PacketFrame) -> Result<LoginStart, ProtocolError> {
    require_packet_id(frame, LOGIN_START_PACKET_ID)?;
    let mut cursor = PacketCursor::new(&frame.payload);
    let name = cursor.read_string(MAX_USERNAME_BYTES)?;
    let profile_id = cursor.read_uuid("login profile id")?;
    cursor.finish()?;
    Ok(LoginStart { name, profile_id })
}

pub fn encode_login_start(value: &LoginStart) -> Result<PacketFrame, ProtocolError> {
    let mut payload = Vec::with_capacity(value.name.len().saturating_add(21));
    write_string(&value.name, MAX_USERNAME_BYTES, &mut payload)?;
    write_uuid(value.profile_id, &mut payload);
    Ok(PacketFrame {
        packet_id: LOGIN_START_PACKET_ID,
        payload,
    })
}

pub fn decode_encryption_response(
    frame: &PacketFrame,
) -> Result<EncryptionResponse, ProtocolError> {
    require_packet_id(frame, LOGIN_ENCRYPTION_RESPONSE_PACKET_ID)?;
    let mut cursor = PacketCursor::new(&frame.payload);
    let shared_secret =
        cursor.read_byte_array(MAX_ENCRYPTED_SECRET_BYTES, "encrypted shared secret")?;
    let verify_token = cursor.read_byte_array(MAX_VERIFY_TOKEN_BYTES, "encrypted verify token")?;
    cursor.finish()?;
    Ok(EncryptionResponse {
        shared_secret,
        verify_token,
    })
}

pub fn encode_encryption_response(
    value: &EncryptionResponse,
) -> Result<PacketFrame, ProtocolError> {
    let mut payload = Vec::new();
    write_byte_array(
        &value.shared_secret,
        MAX_ENCRYPTED_SECRET_BYTES,
        &mut payload,
    )?;
    write_byte_array(&value.verify_token, MAX_VERIFY_TOKEN_BYTES, &mut payload)?;
    Ok(PacketFrame {
        packet_id: LOGIN_ENCRYPTION_RESPONSE_PACKET_ID,
        payload,
    })
}

pub fn decode_login_acknowledged(frame: &PacketFrame) -> Result<LoginAcknowledged, ProtocolError> {
    require_packet_id(frame, LOGIN_ACKNOWLEDGED_PACKET_ID)?;
    PacketCursor::new(&frame.payload).finish()?;
    Ok(LoginAcknowledged)
}

pub fn encode_login_acknowledged(_: LoginAcknowledged) -> PacketFrame {
    PacketFrame {
        packet_id: LOGIN_ACKNOWLEDGED_PACKET_ID,
        payload: Vec::new(),
    }
}

pub fn decode_login_disconnect(frame: &PacketFrame) -> Result<LoginDisconnect, ProtocolError> {
    require_packet_id(frame, LOGIN_DISCONNECT_PACKET_ID)?;
    let mut cursor = PacketCursor::new(&frame.payload);
    let json_reason = cursor.read_string(MAX_LOGIN_JSON_BYTES)?;
    cursor.finish()?;
    Ok(LoginDisconnect { json_reason })
}

pub fn encode_login_disconnect(value: &LoginDisconnect) -> Result<PacketFrame, ProtocolError> {
    let mut payload = Vec::with_capacity(value.json_reason.len().saturating_add(5));
    write_string(&value.json_reason, MAX_LOGIN_JSON_BYTES, &mut payload)?;
    Ok(PacketFrame {
        packet_id: LOGIN_DISCONNECT_PACKET_ID,
        payload,
    })
}

pub fn decode_encryption_request(frame: &PacketFrame) -> Result<EncryptionRequest, ProtocolError> {
    require_packet_id(frame, LOGIN_ENCRYPTION_REQUEST_PACKET_ID)?;
    let mut cursor = PacketCursor::new(&frame.payload);
    let server_id = cursor.read_string(MAX_SERVER_ID_BYTES)?;
    let public_key = cursor.read_byte_array(MAX_PUBLIC_KEY_BYTES, "DER public key")?;
    let verify_token = cursor.read_byte_array(MAX_VERIFY_TOKEN_BYTES, "verify token")?;
    let should_authenticate = cursor.read_bool("should authenticate")?;
    cursor.finish()?;
    Ok(EncryptionRequest {
        server_id,
        public_key,
        verify_token,
        should_authenticate,
    })
}

pub fn encode_encryption_request(value: &EncryptionRequest) -> Result<PacketFrame, ProtocolError> {
    let mut payload = Vec::new();
    write_string(&value.server_id, MAX_SERVER_ID_BYTES, &mut payload)?;
    write_byte_array(&value.public_key, MAX_PUBLIC_KEY_BYTES, &mut payload)?;
    write_byte_array(&value.verify_token, MAX_VERIFY_TOKEN_BYTES, &mut payload)?;
    write_bool(value.should_authenticate, &mut payload);
    Ok(PacketFrame {
        packet_id: LOGIN_ENCRYPTION_REQUEST_PACKET_ID,
        payload,
    })
}

pub fn decode_set_compression(frame: &PacketFrame) -> Result<SetCompression, ProtocolError> {
    require_packet_id(frame, LOGIN_COMPRESSION_PACKET_ID)?;
    let mut cursor = PacketCursor::new(&frame.payload);
    let threshold = cursor.read_varint()?;
    cursor.finish()?;
    Ok(SetCompression { threshold })
}

pub fn encode_set_compression(value: SetCompression) -> PacketFrame {
    let mut payload = Vec::new();
    encode_varint(value.threshold, &mut payload);
    PacketFrame {
        packet_id: LOGIN_COMPRESSION_PACKET_ID,
        payload,
    }
}

pub fn decode_login_finished(frame: &PacketFrame) -> Result<LoginFinished, ProtocolError> {
    require_packet_id(frame, LOGIN_FINISHED_PACKET_ID)?;
    let mut cursor = PacketCursor::new(&frame.payload);
    let profile_id = cursor.read_uuid("login profile id")?;
    let name = cursor.read_string(MAX_USERNAME_BYTES)?;
    let property_count = read_list_length(&mut cursor, MAX_PROFILE_PROPERTIES)?;
    let mut properties = Vec::with_capacity(property_count);
    for _ in 0..property_count {
        let property_name = cursor.read_string(MAX_PROPERTY_NAME_BYTES)?;
        let value = cursor.read_string(MAX_PROPERTY_VALUE_BYTES)?;
        let signature = if cursor.read_bool("profile property signature flag")? {
            Some(cursor.read_string(MAX_PROPERTY_VALUE_BYTES)?)
        } else {
            None
        };
        properties.push(ProfileProperty {
            name: property_name,
            value,
            signature,
        });
    }
    let session_id = cursor.read_uuid("login session id")?;
    cursor.finish()?;
    Ok(LoginFinished {
        profile_id,
        name,
        properties,
        session_id,
    })
}

pub fn encode_login_finished(value: &LoginFinished) -> Result<PacketFrame, ProtocolError> {
    if value.properties.len() > MAX_PROFILE_PROPERTIES {
        return Err(ProtocolError::ListTooLong {
            actual: value.properties.len(),
            maximum: MAX_PROFILE_PROPERTIES,
        });
    }
    let mut payload = Vec::new();
    write_uuid(value.profile_id, &mut payload);
    write_string(&value.name, MAX_USERNAME_BYTES, &mut payload)?;
    encode_varint(value.properties.len() as i32, &mut payload);
    for property in &value.properties {
        write_string(&property.name, MAX_PROPERTY_NAME_BYTES, &mut payload)?;
        write_string(&property.value, MAX_PROPERTY_VALUE_BYTES, &mut payload)?;
        write_bool(property.signature.is_some(), &mut payload);
        if let Some(signature) = &property.signature {
            write_string(signature, MAX_PROPERTY_VALUE_BYTES, &mut payload)?;
        }
    }
    write_uuid(value.session_id, &mut payload);
    Ok(PacketFrame {
        packet_id: LOGIN_FINISHED_PACKET_ID,
        payload,
    })
}

fn read_list_length(cursor: &mut PacketCursor<'_>, maximum: usize) -> Result<usize, ProtocolError> {
    let length = cursor.read_varint()?;
    if length < 0 {
        return Err(ProtocolError::NegativeListLength(length));
    }
    let length = length as usize;
    if length > maximum {
        return Err(ProtocolError::ListTooLong {
            actual: length,
            maximum,
        });
    }
    Ok(length)
}

fn require_packet_id(frame: &PacketFrame, expected: i32) -> Result<(), ProtocolError> {
    if frame.packet_id != expected {
        return Err(ProtocolError::UnexpectedPacketId {
            expected,
            actual: frame.packet_id,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        decode_encryption_request, decode_encryption_response, decode_login_acknowledged,
        decode_login_disconnect, decode_login_finished, decode_login_start, decode_set_compression,
        encode_encryption_request, encode_encryption_response, encode_login_acknowledged,
        encode_login_disconnect, encode_login_finished, encode_login_start, encode_set_compression,
        EncryptionRequest, EncryptionResponse, LoginAcknowledged, LoginDisconnect, LoginFinished,
        LoginStart, ProfileProperty, SetCompression,
    };
    use crate::{encode_varint, PacketFrame, ProtocolError};
    use uuid::Uuid;

    #[test]
    fn login_start_round_trips() {
        let value = LoginStart {
            name: "MythicPlayer".to_owned(),
            profile_id: Uuid::from_u128(1),
        };
        let frame = encode_login_start(&value).unwrap_or(PacketFrame {
            packet_id: -1,
            payload: Vec::new(),
        });
        assert_eq!(decode_login_start(&frame), Ok(value));
    }

    #[test]
    fn encryption_request_and_response_round_trip() {
        let request = EncryptionRequest {
            server_id: String::new(),
            public_key: vec![1, 2, 3],
            verify_token: vec![4, 5, 6, 7],
            should_authenticate: true,
        };
        let response = EncryptionResponse {
            shared_secret: vec![8; 128],
            verify_token: vec![9; 128],
        };

        assert_eq!(
            encode_encryption_request(&request).and_then(|frame| decode_encryption_request(&frame)),
            Ok(request)
        );
        assert_eq!(
            encode_encryption_response(&response)
                .and_then(|frame| decode_encryption_response(&frame)),
            Ok(response)
        );
    }

    #[test]
    fn login_finished_round_trips_26_2_session_id() {
        let value = LoginFinished {
            profile_id: Uuid::from_u128(10),
            name: "MythicPlayer".to_owned(),
            properties: vec![ProfileProperty {
                name: "textures".to_owned(),
                value: "base64-value".to_owned(),
                signature: Some("signature".to_owned()),
            }],
            session_id: Uuid::from_u128(11),
        };
        let frame = encode_login_finished(&value).unwrap_or(PacketFrame {
            packet_id: -1,
            payload: Vec::new(),
        });

        assert_eq!(decode_login_finished(&frame), Ok(value));
    }

    #[test]
    fn disconnect_compression_and_acknowledgement_round_trip() {
        let disconnect = LoginDisconnect {
            json_reason: r#"{"text":"Unsupported version"}"#.to_owned(),
        };
        let compression = SetCompression { threshold: 256 };

        assert_eq!(
            encode_login_disconnect(&disconnect).and_then(|frame| decode_login_disconnect(&frame)),
            Ok(disconnect)
        );
        assert_eq!(
            decode_set_compression(&encode_set_compression(compression)),
            Ok(compression)
        );
        assert_eq!(
            decode_login_acknowledged(&encode_login_acknowledged(LoginAcknowledged)),
            Ok(LoginAcknowledged)
        );
    }

    #[test]
    fn oversized_encrypted_secret_is_rejected_before_body_read() {
        let mut payload = Vec::new();
        encode_varint(513, &mut payload);
        assert_eq!(
            decode_encryption_response(&PacketFrame {
                packet_id: 1,
                payload,
            }),
            Err(ProtocolError::ByteArrayTooLong {
                actual: 513,
                maximum: 512,
            })
        );
    }

    #[test]
    fn excessive_profile_property_count_is_rejected() {
        let mut payload = Vec::new();
        payload.extend_from_slice(Uuid::nil().as_bytes());
        encode_varint(1, &mut payload);
        payload.extend_from_slice(b"a");
        encode_varint(65, &mut payload);

        assert_eq!(
            decode_login_finished(&PacketFrame {
                packet_id: 2,
                payload,
            }),
            Err(ProtocolError::ListTooLong {
                actual: 65,
                maximum: 64,
            })
        );
    }
}
