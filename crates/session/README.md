# Mythicraft Session

Run the protocol `776` development status server with:

```text
cargo run -p mythicraft-session --example status_server -- 127.0.0.1:25565
```

The example accepts the vanilla server-list sequence `Handshake(Status) -> Status Request -> Ping`, returns a JSON status response, echoes the ping payload, and closes the connection.

For `Handshake(Login)`, it validates protocol `776`, decodes Login Start, returns a JSON Login Disconnect explaining that Configuration is unavailable, and closes safely. Wrong login protocol versions receive an immediate Login Disconnect. It is a single-threaded development probe, not the production login or tick runtime.
