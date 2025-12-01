use std::path::PathBuf;
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Serialize)]
struct ElevenLabsRequest {
    text: String,
    model_id: String,
    voice_settings: VoiceSettings,
}

#[derive(Debug, Serialize)]
struct VoiceSettings {
    stability: f32,
    similarity_boost: f32,
    style: f32,
    speed: f32,  // 1.10 for slightly faster
}

pub struct TTSGenerator {
    api_key: String,
    voice_id: String,
    model: String,
    cache_dir: PathBuf,
    client: reqwest::Client,
}

impl TTSGenerator {
    pub fn new(api_key: String, voice_id: String, model: String, cache_dir: PathBuf) -> Self {
        Self {
            api_key,
            voice_id,
            model,
            cache_dir,
            client: reqwest::Client::new(),
        }
    }

    pub async fn generate_speech(&self, text: &str) -> Result<PathBuf, String> {
        println!("Generating TTS: {}", text);

        let file_id = Uuid::new_v4();
        let output_path = self.cache_dir.join(format!("{}.mp3", file_id));

        let request = ElevenLabsRequest {
            text: text.to_string(),
            model_id: self.model.clone(),
            voice_settings: VoiceSettings {
                stability: 0.5,
                similarity_boost: 0.75,
                style: 0.0,
                speed: 1.10,  // 10% faster
            },
        };

        // Call ElevenLabs API with default mp3 format (will convert to 192kbps later)
        let url = format!(
            "https://api.elevenlabs.io/v1/text-to-speech/{}?output_format=mp3_44100_128",
            self.voice_id
        );

        let response = self.client
            .post(&url)
            .header("xi-api-key", &self.api_key)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| format!("ElevenLabs request failed: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("ElevenLabs API error {}: {}", status, body));
        }

        // Get audio bytes
        let audio_bytes = response.bytes().await
            .map_err(|e| format!("Failed to get audio bytes: {}", e))?;

        // Write to temporary file first
        let temp_path = self.cache_dir.join(format!("{}_temp.mp3", file_id));
        tokio::fs::write(&temp_path, audio_bytes).await
            .map_err(|e| format!("Failed to write temp audio file: {}", e))?;

        // Convert to 192kbps using ffmpeg
        println!("Converting TTS audio to 192kbps...");
        let output = tokio::process::Command::new("ffmpeg")
            .args([
                "-i", temp_path.to_str().ok_or("Invalid temp path")?,
                "-b:a", "192k",
                "-ac", "2",
                "-ar", "44100",
                "-y", // Overwrite
                output_path.to_str().ok_or("Invalid output path")?,
            ])
            .output()
            .await
            .map_err(|e| format!("ffmpeg execution failed: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("ffmpeg failed to convert TTS audio: {}", stderr));
        }

        // Delete temp file
        let _ = tokio::fs::remove_file(&temp_path).await;

        // Verify final file exists and has content
        let metadata = tokio::fs::metadata(&output_path).await
            .map_err(|e| format!("Failed to read TTS file metadata: {}", e))?;

        if metadata.len() == 0 {
            return Err(format!("Generated TTS file is empty: {:?}", output_path));
        }

        println!("✓ TTS audio converted: {:?} ({} bytes, 192kbps)", output_path, metadata.len());
        Ok(output_path)
    }
}

