use std::env;
use std::error::Error;
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

use mythicraft_session::{serve_development_io, DevelopmentConfiguration, StatusConfiguration};

fn main() -> Result<(), Box<dyn Error>> {
    let bind_address = env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:25565".to_owned());
    let listener = TcpListener::bind(&bind_address)?;
    println!("Mythicraft protocol 776 development server listening on {bind_address}");

    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                if let Err(error) = handle_client(stream) {
                    eprintln!("status connection failed: {error}");
                }
            }
            Err(error) => eprintln!("failed to accept connection: {error}"),
        }
    }
    Ok(())
}

fn handle_client(mut writer: TcpStream) -> Result<(), Box<dyn Error>> {
    writer.set_read_timeout(Some(Duration::from_secs(10)))?;
    writer.set_write_timeout(Some(Duration::from_secs(10)))?;
    let mut reader = writer.try_clone()?;
    serve_development_io(
        &mut reader,
        &mut writer,
        DevelopmentConfiguration {
            status: StatusConfiguration {
                version_name: "26.2".to_owned(),
                protocol_version: 776,
                motd: "Mythicraft development server".to_owned(),
                max_players: 20,
                online_players: 0,
            },
            login_rejection_message:
                "Login packets were accepted, but Configuration is not implemented.".to_owned(),
        },
    )?;
    Ok(())
}
