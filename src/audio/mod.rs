pub mod cache;
pub mod fetcher;
pub mod tts;
pub mod llm;
pub mod stream;
pub mod metadata;

use std::path::PathBuf;
use crate::RadioSegment;

/// Represents a cached audio file ready to be streamed
#[derive(Debug, Clone)]
pub struct CachedAudio {
    pub segment: RadioSegment,
    pub file_path: PathBuf,
    pub duration_seconds: f64,
}

/// Audio format configuration
#[derive(Debug, Clone)]
pub struct AudioConfig {
    pub sample_rate: u32,
    pub bit_rate: u32,
    pub channels: u16,
    pub format: AudioFormat,
}

#[derive(Debug, Clone)]
pub enum AudioFormat {
    Mp3,
    Opus,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            sample_rate: 44100,
            bit_rate: 192000,
            channels: 2,
            format: AudioFormat::Mp3,
        }
    }
}
