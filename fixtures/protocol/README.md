# Protocol fixtures

The F2 transport slice currently covers canonical Minecraft VarInt encoding, strict VarInt decoding, compressed and uncompressed packet framing, packet-size limits, custom payload headers, keep-alive tracking, protocol 776 handshake/status/login packet IDs, 26.2 login `session_id`, and the handshake/status/login/configuration/play session transitions.

`truncated-varint.hex` is a synthetic one-byte continuation sequence and must be rejected by strict decoding. Streaming frame decoding treats an unfinished prefix or body as incomplete input instead of mutating session or world state.

The `mythicraft-session` development example exposes the vanilla server-list handshake/status/ping sequence over TCP and a fail-closed Login Start probe. Encryption request/response payloads are bounded and encoded, but RSA key generation, session authentication, shared-secret decryption, AES/CFB8 transport encryption, successful login, configuration/play packet IDs, concurrent connection scheduling, and the renderer-specific payload body remain unsupported. Future fixtures must record direction, connection state, protocol version, expected result, and source/license metadata.
