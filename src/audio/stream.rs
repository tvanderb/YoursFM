use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use bytes::Bytes;
use super::cache::AudioCacheManager;

const CHUNK_SIZE: usize = 4096; // 4KB chunks
const BITRATE_KBPS: u32 = 192; // Assumed MP3 bitrate in kbps

// Calculate delay between chunks to match playback speed
// For 192kbps: 192000 bits/sec = 24000 bytes/sec
// With 4KB chunks: 4096 bytes / 24000 bytes/sec = ~170ms per chunk
fn calculate_chunk_delay() -> tokio::time::Duration {
    let bytes_per_second = (BITRATE_KBPS * 1000 / 8) as f64;
    let seconds_per_chunk = CHUNK_SIZE as f64 / bytes_per_second;
    tokio::time::Duration::from_secs_f64(seconds_per_chunk)
}

/// Manages the audio stream, reading from cache and broadcasting to listeners
pub struct AudioStream {
    cache_manager: Arc<AudioCacheManager>,
    broadcast_tx: broadcast::Sender<Bytes>,
}

impl AudioStream {
    pub fn new(cache_manager: Arc<AudioCacheManager>, buffer_size: usize) -> Self {
        let (broadcast_tx, _) = broadcast::channel(buffer_size);

        Self {
            cache_manager,
            broadcast_tx,
        }
    }

    /// Get a receiver for the broadcast stream
    pub fn subscribe(&self) -> broadcast::Receiver<Bytes> {
        self.broadcast_tx.subscribe()
    }

    /// Get the broadcast sender (for HTTP server)
    pub fn get_sender(&self) -> broadcast::Sender<Bytes> {
        self.broadcast_tx.clone()
    }

    /// Get the number of active listeners
    pub fn listener_count(&self) -> usize {
        self.broadcast_tx.receiver_count()
    }

    /// Start streaming audio from the cache
    pub async fn start_streaming(&self) -> Result<(), String> {
        let chunk_delay = calculate_chunk_delay();
        println!("Streaming at {}kbps: {} chunks/sec ({:.0}ms per chunk)",
            BITRATE_KBPS,
            1000.0 / chunk_delay.as_millis() as f64,
            chunk_delay.as_millis());

        loop {
            // Get next cached audio
            println!("[STREAM] Getting next segment from cache...");
            let cached_audio = match self.cache_manager.pop_next().await {
                Some(audio) => {
                    println!("[STREAM] Got segment: {:?}", audio.segment);
                    audio
                }
                None => {
                    // No audio in cache, wait a bit and try again
                    eprintln!("[STREAM] WARNING: No cached audio available, waiting...");
                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                    continue;
                }
            };

            println!("[STREAM] Now streaming: {:?} (duration: {:.1}s)",
                cached_audio.segment, cached_audio.duration_seconds);

            // Verify file exists and has content
            match tokio::fs::metadata(&cached_audio.file_path).await {
                Ok(metadata) => {
                    println!("File size: {} bytes", metadata.len());
                    if metadata.len() == 0 {
                        eprintln!("Error: File is empty: {:?}", cached_audio.file_path);
                        let _ = tokio::fs::remove_file(&cached_audio.file_path).await;
                        continue;
                    }
                }
                Err(e) => {
                    eprintln!("Error: File not found: {:?} - {}", cached_audio.file_path, e);
                    continue;
                }
            }

            // Stream the audio file
            if let Err(e) = self.stream_file(&cached_audio.file_path).await {
                eprintln!("Error streaming file {:?}: {}", cached_audio.file_path, e);
                let _ = tokio::fs::remove_file(&cached_audio.file_path).await;
                continue;
            }

            println!("[STREAM] Finished streaming: {:?} (played for {:.1}s in real-time)",
                cached_audio.segment, cached_audio.duration_seconds);

            // Clean up the file AFTER we start the next one to avoid gaps
            let file_to_cleanup = cached_audio.file_path.clone();

            // Immediately continue to next segment without any delay
            // Cleanup happens asynchronously to not block the stream
            tokio::spawn(async move {
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                let _ = tokio::fs::remove_file(&file_to_cleanup).await;
            });

            println!("[STREAM] Transitioning to next segment immediately...");
            // No delay - immediately loop back to get next segment
        }
    }

    /// Stream a single audio file in chunks
    async fn stream_file(&self, file_path: &std::path::PathBuf) -> Result<(), String> {
        let mut file = File::open(file_path).await
            .map_err(|e| format!("Failed to open file: {}", e))?;

        let mut buffer = vec![0u8; CHUNK_SIZE];
        let mut chunks_sent = 0;

        // Calculate exact delay per chunk for 192kbps streaming
        // 192 kbps = 24,000 bytes/sec
        // 4096 bytes / 24000 bytes/sec = 0.170666 seconds per chunk
        let chunk_delay = tokio::time::Duration::from_micros(170666);

        let stream_start = std::time::Instant::now();
        let mut next_send_time = stream_start;

        println!("Streaming at exactly 192kbps with {:.0}ms per chunk", chunk_delay.as_millis());

        // Send chunks with strict timing
        loop {
            let bytes_read = file.read(&mut buffer).await
                .map_err(|e| format!("Failed to read file: {}", e))?;

            if bytes_read == 0 {
                // End of file
                let elapsed = stream_start.elapsed().as_secs_f64();
                let rate_kbps = (chunks_sent * CHUNK_SIZE) as f64 / elapsed / 1000.0 * 8.0;
                println!("Total chunks sent: {} (avg rate: {:.0} kbps)", chunks_sent, rate_kbps);
                return Ok(());
            }

            // Wait until it's time to send this chunk
            let now = std::time::Instant::now();
            if now < next_send_time {
                tokio::time::sleep(next_send_time - now).await;
            }

            let chunk = Bytes::copy_from_slice(&buffer[..bytes_read]);

            // Broadcast to all listeners
            match self.broadcast_tx.send(chunk) {
                Ok(receivers) => {
                    chunks_sent += 1;
                    if chunks_sent % 100 == 0 {
                        let elapsed = stream_start.elapsed().as_secs_f64();
                        let rate_kbps = (chunks_sent * CHUNK_SIZE) as f64 / elapsed / 1000.0 * 8.0;
                        println!("Streamed {} chunks in {:.1}s (avg rate: {:.0} kbps) to {} listeners",
                            chunks_sent, elapsed, rate_kbps, receivers);
                    }
                }
                Err(e) => {
                    eprintln!("Warning: Broadcast channel full, dropping chunk: {:?}", e);
                    // Continue anyway - don't stop streaming
                }
            }

            // Schedule next chunk send time
            next_send_time += chunk_delay;
        }
    }
}
