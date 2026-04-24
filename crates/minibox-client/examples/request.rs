use minibox_client::DaemonClient;
use minibox_core::protocol::DaemonRequest;
use std::io::Read;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let socket_path = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("MINIBOX_SOCKET_PATH").ok())
        .ok_or("socket path arg or MINIBOX_SOCKET_PATH env var required")?;

    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|e| format!("read request JSON from stdin: {e}"))?;

    let request: DaemonRequest = serde_json::from_str(&input)
        .map_err(|e| format!("parse DaemonRequest JSON from stdin: {e}"))?;

    let client = DaemonClient::with_socket(&socket_path);
    let mut stream = client
        .call(request)
        .await
        .map_err(|e| format!("call daemon on {socket_path}: {e}"))?;

    while let Some(response) = stream
        .next()
        .await
        .map_err(|e| format!("read daemon response: {e}"))?
    {
        println!(
            "{}",
            serde_json::to_string(&response).map_err(|e| format!("encode response JSON: {e}"))?
        );
    }

    Ok(())
}
