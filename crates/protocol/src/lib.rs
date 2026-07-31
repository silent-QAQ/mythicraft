mod compression;
mod frame;
mod handshake;
mod io;
mod login;
mod payload;
mod status;
mod stream;
mod varint;

use thiserror::Error;

pub use compression::{decode_compressed_frame, encode_compressed_frame, CompressionMode};
pub use frame::{decode_frame, encode_frame, DecodeStatus, PacketFrame};
pub use handshake::{
    decode_handshake, encode_handshake, HandshakeNextState, HandshakePacket,
    HANDSHAKE_INTENTION_PACKET_ID,
};
pub use login::{
    decode_encryption_request, decode_encryption_response, decode_login_acknowledged,
    decode_login_disconnect, decode_login_finished, decode_login_start, decode_set_compression,
    encode_encryption_request, encode_encryption_response, encode_login_acknowledged,
    encode_login_disconnect, encode_login_finished, encode_login_start, encode_set_compression,
    EncryptionRequest, EncryptionResponse, LoginAcknowledged, LoginDisconnect, LoginFinished,
    LoginStart, ProfileProperty, SetCompression, LOGIN_ACKNOWLEDGED_PACKET_ID,
    LOGIN_COMPRESSION_PACKET_ID, LOGIN_DISCONNECT_PACKET_ID, LOGIN_ENCRYPTION_REQUEST_PACKET_ID,
    LOGIN_ENCRYPTION_RESPONSE_PACKET_ID, LOGIN_FINISHED_PACKET_ID, LOGIN_START_PACKET_ID,
};
pub use payload::{decode_custom_payload, encode_custom_payload, ChannelId, CustomPayload};
pub use status::{
    decode_status_ping, decode_status_pong, decode_status_request, decode_status_response,
    encode_status_ping, encode_status_pong, encode_status_request, encode_status_response,
    StatusPing, StatusPong, StatusRequest, StatusResponse, STATUS_PING_REQUEST_PACKET_ID,
    STATUS_PONG_RESPONSE_PACKET_ID, STATUS_REQUEST_PACKET_ID, STATUS_RESPONSE_PACKET_ID,
};
pub use stream::FrameDecoder;
pub(crate) use varint::try_decode_varint;
pub use varint::{decode_varint, encode_varint, DecodedVarInt};

pub const MAX_PACKET_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_VARINT_BYTES: usize = 5;
pub const MAX_CHANNEL_ID_BYTES: usize = 128;
pub const MAX_CUSTOM_PAYLOAD_BYTES: usize = 60 * 1024;
pub const MAX_SERVER_ADDRESS_BYTES: usize = 255;
pub const MAX_STATUS_JSON_BYTES: usize = 32_767;
pub const MAX_STREAM_BUFFER_BYTES: usize = MAX_PACKET_BYTES + MAX_VARINT_BYTES;

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ProtocolError {
    #[error("packet exceeds the {MAX_PACKET_BYTES}-byte limit: {actual} bytes")]
    PacketTooLarge { actual: usize },
    #[error("VarInt ended before a terminating byte")]
    TruncatedVarInt,
    #[error("VarInt exceeds {MAX_VARINT_BYTES} bytes or the i32 range")]
    VarIntTooLong,
    #[error("VarInt uses a non-canonical encoding")]
    NonCanonicalVarInt,
    #[error("packet length cannot be negative: {0}")]
    NegativePacketLength(i32),
    #[error("packet body must contain a packet id")]
    EmptyPacket,
    #[error("packet id cannot be negative: {0}")]
    NegativePacketId(i32),
    #[error("compression threshold exceeds the packet limit: {0}")]
    InvalidCompressionThreshold(usize),
    #[error("declared uncompressed packet length cannot be negative: {0}")]
    NegativeUncompressedLength(i32),
    #[error("compressed packet length {declared} is below threshold {threshold}")]
    CompressedBelowThreshold { declared: usize, threshold: usize },
    #[error("uncompressed packet length {actual} is not below threshold {threshold}")]
    UncompressedAboveThreshold { actual: usize, threshold: usize },
    #[error("zlib compression or decompression failed: {0}")]
    CompressionFailure(String),
    #[error("decompressed length mismatch: declared {declared}, actual {actual}")]
    DecompressedLengthMismatch { declared: usize, actual: usize },
    #[error("compressed stream has trailing bytes: consumed {consumed}, actual {actual}")]
    CompressedTrailingBytes { consumed: usize, actual: usize },
    #[error("channel length must be within 1..={MAX_CHANNEL_ID_BYTES} bytes, got {0}")]
    InvalidChannelLength(i32),
    #[error("invalid channel id: {0}")]
    InvalidChannelId(String),
    #[error("channel id is not valid UTF-8")]
    InvalidChannelUtf8,
    #[error("schema version must be within 1..={maximum}, got {actual}")]
    InvalidSchemaVersion { actual: i32, maximum: u16 },
    #[error("payload length cannot be negative: {0}")]
    NegativePayloadLength(i32),
    #[error("custom payload exceeds the {MAX_CUSTOM_PAYLOAD_BYTES}-byte limit: {actual} bytes")]
    CustomPayloadTooLarge { actual: usize },
    #[error("custom payload length mismatch: declared {declared}, actual {actual}")]
    PayloadLengthMismatch { declared: usize, actual: usize },
    #[error("custom payload header or body is truncated")]
    TruncatedCustomPayload,
    #[error("string length cannot be negative: {0}")]
    NegativeStringLength(i32),
    #[error("string exceeds the {maximum}-byte limit: {actual} bytes")]
    StringTooLong { actual: usize, maximum: usize },
    #[error("string is not valid UTF-8")]
    InvalidStringUtf8,
    #[error("field {field} is truncated: needed {needed} bytes, remaining {remaining}")]
    TruncatedField {
        field: &'static str,
        needed: usize,
        remaining: usize,
    },
    #[error("unexpected packet id {actual}; expected {expected}")]
    UnexpectedPacketId { expected: i32, actual: i32 },
    #[error("packet contains {remaining} trailing bytes")]
    TrailingPacketData { remaining: usize },
    #[error("invalid handshake next state {0}; expected 1 (status) or 2 (login)")]
    InvalidHandshakeNextState(i32),
    #[error("invalid boolean byte {0}; expected 0 or 1")]
    InvalidBoolean(u8),
    #[error("byte array length cannot be negative: {0}")]
    NegativeByteArrayLength(i32),
    #[error("byte array exceeds the {maximum}-byte limit: {actual} bytes")]
    ByteArrayTooLong { actual: usize, maximum: usize },
    #[error("list length cannot be negative: {0}")]
    NegativeListLength(i32),
    #[error("list exceeds the {maximum}-item limit: {actual} items")]
    ListTooLong { actual: usize, maximum: usize },
    #[error("stream buffer exceeds the {maximum}-byte limit: {actual} bytes")]
    StreamBufferTooLarge { actual: usize, maximum: usize },
    #[error("cannot change compression mode while {buffered} bytes remain buffered")]
    CompressionModeChangeWithBufferedData { buffered: usize },
}
