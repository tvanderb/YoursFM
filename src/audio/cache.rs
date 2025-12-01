use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Mutex, Semaphore};
use crate::RadioSegment;
use super::{CachedAudio, AudioConfig};
use super::fetcher::AudioFetcher;

/// Manages pre-caching of audio segments
pub struct AudioCacheManager {
    cache_queue: Arc<Mutex<VecDeque<CachedAudio>>>,
    max_cache_size: usize,
    audio_config: AudioConfig,
    fetcher: Arc<AudioFetcher>,
    /// Semaphore to limit concurrent fetches
    fetch_semaphore: Arc<Semaphore>,
    /// Debug mode: skip songs and only play callouts
    callouts_only: bool,
}

impl AudioCacheManager {
    pub fn new(
        max_cache_size: usize,
        audio_config: AudioConfig,
        fetcher: Arc<AudioFetcher>,
        callouts_only: bool,
    ) -> Self {
        Self {
            cache_queue: Arc::new(Mutex::new(VecDeque::new())),
            max_cache_size,
            audio_config,
            fetcher,
            fetch_semaphore: Arc::new(Semaphore::new(2)), // Max 2 concurrent fetches
            callouts_only,
        }
    }

    /// Get the next cached audio segment (remove from queue)
    pub async fn pop_next(&self) -> Option<CachedAudio> {
        let mut queue = self.cache_queue.lock().await;
        queue.pop_front()
    }

    /// Check how many segments are currently cached
    pub async fn cached_count(&self) -> usize {
        let queue = self.cache_queue.lock().await;
        queue.len()
    }

    /// Pre-cache a segment asynchronously
    pub async fn cache_segment(&self, segment: RadioSegment) -> Result<(), String> {
        // Skip songs in callouts-only debug mode
        if self.callouts_only {
            match &segment {
                RadioSegment::Song(track) => {
                    println!("[DEBUG] ⏭️  Skipping song: {} - {}", track.artist, track.name);
                    return Err("Skipping song in callouts-only mode".to_string());
                }
                RadioSegment::Show(track) => {
                    println!("[DEBUG] ⏭️  Skipping show: {}", track.name);
                    return Err("Skipping show in callouts-only mode".to_string());
                }
                _ => {
                    // Continue with callouts
                    println!("[DEBUG] 🎤 Processing callout: {:?}", segment);
                }
            }
        }

        // Acquire semaphore permit to limit concurrent fetches
        let _permit = self.fetch_semaphore.acquire().await.map_err(|e| e.to_string())?;

        let cached_audio = match &segment {
            RadioSegment::Song(track) | RadioSegment::Show(track) => {
                // Fetch from YouTube
                let file_path = self.fetcher.fetch_youtube_audio(
                    &track.name,
                    &track.artist,
                    &track.album_name,
                    &self.audio_config,
                ).await?;

                // Get duration (we'll implement this)
                let duration = self.get_audio_duration(&file_path).await?;

                CachedAudio {
                    segment,
                    file_path,
                    duration_seconds: duration,
                }
            }
            _ => {
                // Generate TTS audio for callouts, news, weather, etc.
                let file_path = self.fetcher.generate_tts_audio(
                    &segment,
                    &self.audio_config,
                ).await?;

                let duration = self.get_audio_duration(&file_path).await?;

                CachedAudio {
                    segment,
                    file_path,
                    duration_seconds: duration,
                }
            }
        };

        // Add to cache queue
        let mut queue = self.cache_queue.lock().await;
        queue.push_back(cached_audio);

        // Clean up old cache if needed
        while queue.len() > self.max_cache_size {
            if let Some(old) = queue.pop_front() {
                // Delete old file
                let _ = tokio::fs::remove_file(old.file_path).await;
            }
        }

        Ok(())
    }

    /// Get audio file duration using ffprobe
    async fn get_audio_duration(&self, file_path: &PathBuf) -> Result<f64, String> {
        let output = tokio::process::Command::new("ffprobe")
            .args([
                "-v", "error",
                "-show_entries", "format=duration",
                "-of", "default=noprint_wrappers=1:nokey=1",
                file_path.to_str().ok_or("Invalid path")?
            ])
            .output()
            .await
            .map_err(|e| format!("ffprobe failed: {}", e))?;

        let duration_str = String::from_utf8_lossy(&output.stdout);
        duration_str.trim().parse::<f64>()
            .map_err(|e| format!("Failed to parse duration: {}", e))
    }

    /// Clean up all cached files
    pub async fn cleanup_all(&self) -> Result<(), String> {
        let mut queue = self.cache_queue.lock().await;
        while let Some(cached) = queue.pop_front() {
            let _ = tokio::fs::remove_file(cached.file_path).await;
        }
        Ok(())
    }
}
