use axum::{
    Router,
    response::{Response, IntoResponse},
    extract::State,
    http::{StatusCode, header},
};
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use bytes::Bytes;

/// HTTP server state
#[derive(Clone)]
pub struct ServerState {
    pub broadcast_tx: broadcast::Sender<Bytes>,
}

/// Handler for the /stream endpoint
async fn stream_handler(State(state): State<ServerState>) -> impl IntoResponse {
    let receiver = state.broadcast_tx.subscribe();

    // Use tokio_stream's BroadcastStream which properly handles async waking
    let stream = BroadcastStream::new(receiver)
        .map(|result| -> Result<Bytes, std::io::Error> {
            match result {
                Ok(bytes) => Ok(bytes),
                Err(e) => {
                    eprintln!("Warning: Stream error: {:?}", e);
                    // Return empty bytes on error to continue stream
                    Ok(Bytes::new())
                }
            }
        });

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "audio/mpeg")
        .header(header::CACHE_CONTROL, "no-cache, no-store, must-revalidate")
        .header(header::PRAGMA, "no-cache")
        .header(header::EXPIRES, "0")
        .header("X-Content-Type-Options", "nosniff")
        .body(axum::body::Body::from_stream(stream))
        .unwrap()
}

/// Handler for the root endpoint
async fn root_handler() -> impl IntoResponse {
    let html = r#"
<!DOCTYPE html>
<html>
<head>
    <title>YoursFM - Streaming Radio</title>
    <style>
        body {
            font-family: Arial, sans-serif;
            max-width: 800px;
            margin: 50px auto;
            padding: 20px;
            background: #1a1a1a;
            color: #fff;
        }
        h1 { color: #4CAF50; }
        audio {
            width: 100%;
            margin: 20px 0;
        }
        .info {
            background: #2a2a2a;
            padding: 15px;
            border-radius: 5px;
            margin: 10px 0;
        }
    </style>
</head>
<body>
    <h1>📻 YoursFM - Procedurally Generated Radio</h1>
    <div class="info">
        <p>Listen to your personalized radio station, streaming live!</p>
    </div>
    <audio controls autoplay>
        <source src="/stream" type="audio/mpeg">
        Your browser does not support the audio element.
    </audio>
    <div class="info">
        <h3>Stream URL:</h3>
        <code>http://localhost:3000/stream</code>
    </div>
</body>
</html>
    "#;

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html")
        .body(html.to_string())
        .unwrap()
}

/// Create and configure the HTTP server
pub fn create_server(broadcast_tx: broadcast::Sender<Bytes>) -> Router {
    let state = ServerState { broadcast_tx };

    Router::new()
        .route("/", axum::routing::get(root_handler))
        .route("/stream", axum::routing::get(stream_handler))
        .with_state(state)
}

/// Start the HTTP server
pub async fn start_server(
    broadcast_tx: broadcast::Sender<Bytes>,
    port: u16,
) -> Result<(), String> {
    let app = create_server(broadcast_tx);
    let addr = format!("0.0.0.0:{}", port);

    println!("🎵 YoursFM HTTP Server starting on http://{}", addr);
    println!("📻 Stream URL: http://localhost:{}/stream", port);

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("Failed to bind to {}: {}", addr, e))?;

    axum::serve(listener, app)
        .await
        .map_err(|e| format!("Server error: {}", e))?;

    Ok(())
}
