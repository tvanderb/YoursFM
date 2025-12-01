use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;
use crate::{RadioSegment, StationConfiguration};
use super::AudioConfig;
use super::llm::LLMGenerator;
use super::tts::TTSGenerator;

/// Handles fetching audio from YouTube and generating TTS
pub struct AudioFetcher {
    cache_dir: PathBuf,
    llm: Arc<LLMGenerator>,
    tts: Arc<TTSGenerator>,
    station: StationConfiguration,
}

impl AudioFetcher {
    pub fn new(
        cache_dir: PathBuf,
        llm: Arc<LLMGenerator>,
        tts: Arc<TTSGenerator>,
        station: StationConfiguration,
    ) -> Self {
        Self {
            cache_dir,
            llm,
            tts,
            station,
        }
    }

    /// Fetch audio from YouTube using yt-dlp
    pub async fn fetch_youtube_audio(
        &self,
        track_name: &str,
        artist: &str,
        album_name: &str,
        audio_config: &AudioConfig,
    ) -> Result<PathBuf, String> {
        let search_query = format!("{} - {} - {}", track_name, artist, album_name);
        let file_id = Uuid::new_v4();
        let output_path = self.cache_dir.join(format!("{}.mp3", file_id));

        println!("Fetching from YouTube: {}", search_query);

        // Use yt-dlp to search and download first result
        // Force 192kbps CBR for consistent streaming
        let output = tokio::process::Command::new("yt-dlp")
            .args([
                &format!("ytsearch1:{}", search_query),
                "-x", // Extract audio
                "--audio-format", "mp3",
                "--postprocessor-args", "ffmpeg:-b:a 192k -ac 2 -ar 44100", // Force 192kbps CBR, stereo, 44.1kHz
                "-o", output_path.to_str().ok_or("Invalid path")?,
                "--no-playlist",
            ])
            .output()
            .await
            .map_err(|e| format!("yt-dlp execution failed (is yt-dlp installed?): {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            return Err(format!("yt-dlp failed to fetch: {}\nStdout: {}\nStderr: {}", search_query, stdout, stderr));
        }

        // Verify file exists and has content
        if !output_path.exists() {
            return Err(format!("Downloaded file not found: {:?}", output_path));
        }

        // Wait a moment for file system to sync
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        // Verify file has content
        let metadata = tokio::fs::metadata(&output_path).await
            .map_err(|e| format!("Failed to read file metadata: {}", e))?;

        if metadata.len() == 0 {
            return Err(format!("Downloaded file is empty: {:?}", output_path));
        }

        println!("Downloaded: {:?} ({} bytes)", output_path, metadata.len());
        Ok(output_path)
    }

    /// Generate TTS audio for callouts, news, weather, etc.
    pub async fn generate_tts_audio(
        &self,
        segment: &RadioSegment,
        _audio_config: &AudioConfig,
    ) -> Result<PathBuf, String> {
        println!("Generating TTS for: {:?}", segment);

        // Step 1: Generate text using LLM
        let text = self.llm.generate_callout_text(segment, &self.station).await?;

        println!("✓ LLM generated text: \"{}\"", text);

        // Step 2: Generate speech using ElevenLabs TTS (192kbps MP3 at 1.10 speed)
        let audio_path = self.tts.generate_speech(&text).await?;

        println!("✓ TTS generated audio: {:?}", audio_path);
        Ok(audio_path)
    }

}
