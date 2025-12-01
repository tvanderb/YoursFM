mod spotify;
mod audio;
mod server;

use std::collections::LinkedList;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};
use sqlx::migrate::MigrateDatabase;
use sqlx::sqlite::SqlitePoolOptions;
use rand::Rng;
use rand::seq::SliceRandom;

const DEFAULT_CONFIGURATION_FILEPATH: &str = "./config.yaml";

#[derive(Debug)]
enum LogVerbosity {
    OFF,
    MINIMAL,
    INCREASED,
    FULL
}

#[derive(Debug, Clone)]
struct CalloutContext {
    last_song: Option<YoursFMTrack>,
    next_song: Option<YoursFMTrack>,
}

#[derive(Debug, Clone)]
enum RadioSegment {
    // Callouts with full context (last AND next songs)
    NextSongCallout(CalloutContext),
    LastSongCallout(CalloutContext),
    RadioCallout(CalloutContext),
    ShowIntroduction(CalloutContext),
    NextSongCalloutDetailed(CalloutContext),
    NextSongCalloutHistory(CalloutContext),
    NewsReport(CalloutContext),
    WeatherReport(CalloutContext),
    Song(YoursFMTrack),
    Show(YoursFMTrack),
    ShowClose(CalloutContext),
}

#[derive(Debug, Serialize, Deserialize)]
struct StationLocation {
    name: String,
    airport: String
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct StationConfiguration {
    code: String,
    name: String,
    frequency: String,
    channel: String,
}

impl StationConfiguration {
    /// Get formatted station identifier: "TVAN-AM 840"
    pub fn identifier(&self) -> String {
        format!("{}-{} {}", self.code, self.channel, self.frequency)
    }

    /// Get full station description: "Talon's Radio, TVAN-AM 840"
    pub fn full_name(&self) -> String {
        format!("{}, {}", self.name, self.identifier())
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct SpotifyConfiguration {
    songs_playlist_id: String,
    shows_playlist_id: String,
    client_id: String,
    client_secret: String
}

#[derive(Debug, Serialize, Deserialize)]
struct DatabaseConfiguration {
    location: String
}

#[derive(Debug, Serialize, Deserialize)]
struct LLMConfiguration {
    openrouter_api_key: String,
    model: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct TTSConfiguration {
    elevenlabs_api_key: String,
    voice_id: String,
    model: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct MetadataConfiguration {
    lastfm_api_key: String,
    musicbrainz_contact: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Configuration {
    spotify: SpotifyConfiguration,
    station: StationConfiguration,
    database: DatabaseConfiguration,
    llm: LLMConfiguration,
    tts: TTSConfiguration,
    metadata: MetadataConfiguration,
}

#[derive(Debug, Clone)]
struct YoursFMTrack {
    id: String,
    name: String,
    artist: String,
    album_name: String,
    album_release_date: String
}

pub fn generate_track_id(name: String, artist: String, album_name: String, album_release_date: String) -> String {
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, format!("{}-{}-{}-{}", name, artist, album_name, album_release_date).as_bytes()).to_string()
}

#[tokio::main]
async fn main() {
    let command_argument_matches = clap::command!()
        .arg(clap::arg!([name] "YoursFM"))
        .arg(
            clap::arg!(-c --config <FILE> "Path to config.json file")
                .env("YOURSFM_CONFIG_FILE")
                .required(false)
        )
        .arg(
            clap::arg!(-v --verbose ... "Increase log verbosity")
                .env("YOURSFM_VERBOSITY")
                .required(false)
        )
        .arg(
            clap::arg!(--"callouts-only" "Debug mode: only play callouts, skip songs")
                .env("YOURSFM_CALLOUTS_ONLY")
                .required(false)
                .action(clap::ArgAction::SetTrue)
        )
        .get_matches();

    // Parse verbosity level
    let verbosity: LogVerbosity = match command_argument_matches.get_one::<u8>("verbose").expect("Verbosity is defaulted.") {
        0 => LogVerbosity::OFF,
        1 => LogVerbosity::MINIMAL,
        2 => LogVerbosity::INCREASED,
        3 => LogVerbosity::FULL,
        _ => LogVerbosity::OFF
    };

    // Parse callouts-only debug mode
    let callouts_only = command_argument_matches.get_flag("callouts-only");
    if callouts_only {
        println!("⚠️  DEBUG MODE: Callouts-only enabled - songs will be skipped!");
    }

    // Load configuration filepath then read and serialize configuration
    let config: Configuration = {
        let configuration_filepath: std::path::PathBuf = if let Some(configuration_filepath) = command_argument_matches.get_one::<&str>("config") {
            std::path::PathBuf::from(configuration_filepath)
        } else {
            std::path::PathBuf::from(DEFAULT_CONFIGURATION_FILEPATH)
        };

        println!("Using configuration file: {}", configuration_filepath.display()); // TODO: Change to logging system hook

        let configuration_data = match std::fs::read_to_string(&configuration_filepath) {
            Ok(configuration) => configuration,
            Err(error) => panic!("Configuration file read error: {:?}.", error) // TODO: Change to logging system hook
        };

        match serde_yaml::from_str(&configuration_data) {
            Ok(configuration) => configuration,
            Err(error) => panic!("Configuration deserialization error: {:?}.", error) // TODO: Change to logging system hook
        }
    };

    // Check and create database if it doesn't already exist
    if !match Sqlite::database_exists(config.database.location.as_str()).await {
        Ok(exists) => exists,
        Err(error) => panic!("Database exists check error: {:?}.", error)
    } {
        match Sqlite::create_database(config.database.location.as_str()).await {
            Ok(_) => {}
            Err(error) => panic!("Database creation error: {:?}.", error)
        }
    }

    // Setup database connection pool
    let sqlite_connection_pool = match SqlitePoolOptions::new()
        .max_connections(3)
        .connect(format!("sqlite://{}", config.database.location).as_str()).await {
        Ok(pool) => pool,
        Err(error) => panic!("SQLite database connection error: {:?}.", error)
    };

    // Create database schema
    let create_tracks_table_query = "
            CREATE TABLE IF NOT EXISTS tracks (
                id TEXT NOT NULL,
                type INTEGER NOT NULL,
                name TEXT NOT NULL,
                artist TEXT NOT NULL,
                album_name TEXT NOT NULL,
                album_release_date TEXT NOT NULL
            );
        ";

    match sqlx::query(create_tracks_table_query).execute(&sqlite_connection_pool).await {
        Ok(_) => {}
        Err(error) => panic!("SQLite database connection error: {:?}.", error)
    };

    async fn refresh_tracks(config: &Configuration, sqlite_connection_pool: Pool<Sqlite>) -> (Vec<YoursFMTrack>, Vec<YoursFMTrack>) {
        async fn insert_tracks(tracks: &Vec<YoursFMTrack>, tracks_type: i8, pool: &Pool<Sqlite>) {
            for x in tracks {
                match sqlx::query("INSERT INTO tracks (id, type, name, artist, album_name, album_release_date) VALUES (?, ?, ?, ?, ?, ?)")
                    .bind(&x.id)
                    .bind(&tracks_type)
                    .bind(&x.name)
                    .bind(&x.artist)
                    .bind(&x.album_name)
                    .bind(&x.album_release_date)
                    .execute(pool)
                    .await {
                    Ok(_) => {}
                    Err(error) => panic!("Error inserting song to database: {:?}.", error)
                }
            }
        }

        let songs = spotify::get_all_yoursfm_tracks_from_spotify_playlist(config.spotify.client_id.clone(), config.spotify.client_secret.clone(), config.spotify.songs_playlist_id.clone()).await;
        match sqlx::query("DELETE FROM tracks WHERE type = 0;").execute(&sqlite_connection_pool).await {
            Ok(_) => {}
            Err(error) => panic!("Error clearing song-type tracks from database: {:?}.", error)
        }
        insert_tracks(&songs, 0, &sqlite_connection_pool).await;

        let shows = spotify::get_all_yoursfm_tracks_from_spotify_playlist(config.spotify.client_id.clone(), config.spotify.client_secret.clone(), config.spotify.shows_playlist_id.clone()).await;
        match sqlx::query("DELETE FROM tracks WHERE type = 1;").execute(&sqlite_connection_pool).await {
            Ok(_) => {}
            Err(error) => panic!("Error clearing song-type tracks from database: {:?}.", error)
        }
        insert_tracks(&shows, 1, &sqlite_connection_pool).await;

        (songs, shows)
    }

    // Check for required dependencies
    println!("Checking dependencies...");

    let yt_dlp_check = tokio::process::Command::new("yt-dlp")
        .arg("--version")
        .output()
        .await;

    match yt_dlp_check {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout);
            println!("✓ yt-dlp found: {}", version.trim());
        }
        _ => {
            eprintln!("✗ yt-dlp not found!");
            eprintln!("  Please install yt-dlp:");
            eprintln!("  Windows: choco install yt-dlp");
            eprintln!("  Or download from: https://github.com/yt-dlp/yt-dlp/releases");
            panic!("Missing dependency: yt-dlp");
        }
    }

    let ffmpeg_check = tokio::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .await;

    match ffmpeg_check {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout);
            let first_line = version.lines().next().unwrap_or("unknown");
            println!("✓ ffmpeg found: {}", first_line);
        }
        _ => {
            eprintln!("✗ ffmpeg not found!");
            eprintln!("  Please install ffmpeg:");
            eprintln!("  Windows: choco install ffmpeg");
            eprintln!("  Or download from: https://ffmpeg.org/download.html");
            panic!("Missing dependency: ffmpeg");
        }
    }

    println!("All dependencies satisfied!\n");

    let (mut songs, mut shows) = refresh_tracks(&config, sqlite_connection_pool).await;

    // Shuffle the track and show pools for variety
    let mut rng = rand::thread_rng();
    songs.shuffle(&mut rng);
    shows.shuffle(&mut rng);

    // Create cache directory
    let cache_dir = std::path::PathBuf::from("./cache");
    if !cache_dir.exists() {
        std::fs::create_dir_all(&cache_dir)
            .expect("Failed to create cache directory");
    }

    // Initialize audio system
    let audio_config = audio::AudioConfig::default();

    // Create metadata fetcher (Last.fm + MusicBrainz)
    println!("Initializing metadata fetcher (Last.fm + MusicBrainz)...");
    let metadata_fetcher = std::sync::Arc::new(
        audio::metadata::MetadataFetcher::new(
            config.metadata.lastfm_api_key.clone(),
            config.metadata.musicbrainz_contact.clone(),
        )
    );

    // Create LLM generator (OpenRouter) with metadata fetcher
    println!("Initializing LLM (OpenRouter)...");
    let llm = std::sync::Arc::new(
        audio::llm::LLMGenerator::new(
            config.llm.openrouter_api_key.clone(),
            config.llm.model.clone(),
            metadata_fetcher,
        )
    );

    // Create TTS generator (ElevenLabs)
    println!("Initializing TTS (ElevenLabs)...");
    let tts = std::sync::Arc::new(
        audio::tts::TTSGenerator::new(
            config.tts.elevenlabs_api_key.clone(),
            config.tts.voice_id.clone(),
            config.tts.model.clone(),
            cache_dir.clone(),
        )
    );

    // Create audio fetcher with LLM and TTS
    let fetcher = std::sync::Arc::new(
        audio::fetcher::AudioFetcher::new(
            cache_dir.clone(),
            llm,
            tts,
            config.station.clone(),
        )
    );

    // Create cache manager
    let cache_manager = std::sync::Arc::new(
        audio::cache::AudioCacheManager::new(10, audio_config.clone(), fetcher, callouts_only)
    );

    // Create audio stream with large buffer to prevent overflow
    // 500 chunks = ~2MB = ~42 seconds of buffer at 192kbps
    let audio_stream = std::sync::Arc::new(
        audio::stream::AudioStream::new(cache_manager.clone(), 500)
    );

    // Pre-populate initial segments
    let segment_queue = populate_radio_segments(&mut songs, &mut shows);
    println!("Generated {} initial segments", segment_queue.len());

    // Start pre-caching task
    let cache_manager_clone = cache_manager.clone();
    let mut segment_queue_clone = segment_queue.clone();
    let cache_threshold = if callouts_only { 3 } else { 5 }; // Lower threshold for callouts-only mode
    let pre_cache_handle = tokio::spawn(async move {
        loop {
            // Check cache status
            let cached_count = cache_manager_clone.cached_count().await;

            if cached_count < cache_threshold {
                // Need to cache more segments
                if let Some(segment) = segment_queue_clone.pop_front() {
                    println!("Pre-caching segment: {:?}", segment);
                    match cache_manager_clone.cache_segment(segment.clone()).await {
                        Ok(_) => {
                            println!("✓ Segment cached successfully");
                        }
                        Err(e) => {
                            eprintln!("✗ Failed to cache segment: {}", e);
                            eprintln!("  Skipping this segment and continuing to next...");
                            // Continue immediately to try next segment
                            continue;
                        }
                    }
                } else {
                    println!("Segment queue exhausted, generating more...");
                    // TODO: Generate more segments dynamically
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                }
            } else {
                // Cache is full enough, wait a bit
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            }
        }
    });

    // Wait for at least N segments to be cached before streaming
    // In callouts-only mode, we only need 1 segment since callouts are short
    let min_cache_count = if callouts_only { 1 } else { 2 };
    println!("Waiting for initial segments to be cached (need at least {})...", min_cache_count);

    let mut wait_iterations = 0;
    while cache_manager.cached_count().await < min_cache_count {
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        wait_iterations += 1;

        // Debug output every 10 iterations (5 seconds)
        if wait_iterations % 10 == 0 {
            let count = cache_manager.cached_count().await;
            println!("Still waiting... (cached: {}/{})", count, min_cache_count);
        }

        // Safety timeout after 30 seconds
        if wait_iterations > 60 {
            eprintln!("⚠️  Timeout waiting for cache to populate. Starting anyway with {} segments.", cache_manager.cached_count().await);
            break;
        }
    }
    println!("Initial cache populated, starting stream...\n");

    // Start streaming task
    let audio_stream_clone = audio_stream.clone();
    tokio::spawn(async move {
        if let Err(e) = audio_stream_clone.start_streaming().await {
            eprintln!("Streaming error: {}", e);
        }
    });

    // Get broadcast sender for HTTP server
    let broadcast_tx = audio_stream.get_sender();

    // Start HTTP server
    println!("\n🚀 Starting YoursFM streaming server...\n");
    server::start_server(broadcast_tx, 3000).await
        .expect("Failed to start HTTP server");
}

// Helper to peek at next song without removing it
fn peek_next_song(track_pool: &Vec<YoursFMTrack>) -> Option<YoursFMTrack> {
    track_pool.last().cloned()
}

fn populate_radio_segments(
    track_pool: &mut Vec<YoursFMTrack>,
    show_pool: &mut Vec<YoursFMTrack>,
) -> LinkedList<RadioSegment> {
    let mut segments: LinkedList<RadioSegment> = LinkedList::new();
    let mut rng = rand::thread_rng();

    // Track last song for context
    let mut last_song: Option<YoursFMTrack> = None;

    // Track the last non-song segment type to prevent duplicates
    #[derive(PartialEq, Clone, Copy)]
    enum LastSegmentType {
        None,
        RadioCallout,
        NewsReport,
        WeatherReport,
    }
    let mut last_segment_type = LastSegmentType::None;

    let mut i = 0;
    while i < 64 {
        let roll: f32 = rng.r#gen();

        if roll < 0.75 {
            // 75% chance: Song block (bias towards songs)
            let songs_in_block = if rng.r#gen::<f32>() < 0.4 {
                rng.gen_range(2..=4)
            } else {
                1
            };

            // Decide on callout before songs
            let callout_roll: f32 = rng.r#gen();
            if callout_roll < 0.55 {
                let next_song = peek_next_song(track_pool);
                let context = CalloutContext {
                    last_song: last_song.clone(),
                    next_song: next_song.clone(),
                };

                let callout_type: f32 = rng.r#gen();
                if callout_type < 0.6 {
                    segments.push_back(RadioSegment::NextSongCallout(context));
                } else if callout_type < 0.8 {
                    segments.push_back(RadioSegment::NextSongCalloutDetailed(context));
                } else {
                    segments.push_back(RadioSegment::NextSongCalloutHistory(context));
                }
                i += 1;
                // Song callouts don't reset the duplicate tracker (they're unique per song)
            }

            // Add the songs
            for j in 0..songs_in_block {
                if i >= 64 {
                    break;
                }
                if let Some(track) = track_pool.pop() {
                    segments.push_back(RadioSegment::Song(track.clone()));
                    last_song = Some(track); // Update last song
                    i += 1;
                    // Songs reset the duplicate tracker - allows callouts after songs
                    last_segment_type = LastSegmentType::None;
                }

                // Small chance of radio callout between songs in a block
                // But only if last wasn't a radio callout
                if j < songs_in_block - 1
                    && rng.r#gen::<f32>() < 0.1
                    && last_segment_type != LastSegmentType::RadioCallout
                {
                    let context = CalloutContext {
                        last_song: last_song.clone(),
                        next_song: peek_next_song(track_pool),
                    };
                    segments.push_back(RadioSegment::RadioCallout(context));
                    i += 1;
                    last_segment_type = LastSegmentType::RadioCallout;
                }
            }

            // Less common: last_song_callout after songs (25% chance)
            // ALWAYS followed by next_song_callout for smooth transitions
            if rng.r#gen::<f32>() < 0.25 && i < 64 {
                let context = CalloutContext {
                    last_song: last_song.clone(),
                    next_song: peek_next_song(track_pool),
                };
                segments.push_back(RadioSegment::LastSongCallout(context));
                i += 1;

                // ALWAYS add a NextSongCallout after LastSongCallout
                if i < 64 {
                    let next_context = CalloutContext {
                        last_song: last_song.clone(),
                        next_song: peek_next_song(track_pool),
                    };
                    segments.push_back(RadioSegment::NextSongCallout(next_context));
                    i += 1;
                }
            }

        } else if roll < 0.85 {
            // 10% chance: News report (only if last wasn't news)
            if last_segment_type != LastSegmentType::NewsReport {
                let context = CalloutContext {
                    last_song: last_song.clone(),
                    next_song: peek_next_song(track_pool),
                };
                segments.push_back(RadioSegment::NewsReport(context));
                i += 1;
                last_segment_type = LastSegmentType::NewsReport;
            }
            // If duplicate, skip and let next iteration pick something else

        } else if roll < 0.93 {
            // 8% chance: Weather report (only if last wasn't weather)
            if last_segment_type != LastSegmentType::WeatherReport {
                let context = CalloutContext {
                    last_song: last_song.clone(),
                    next_song: peek_next_song(track_pool),
                };
                segments.push_back(RadioSegment::WeatherReport(context));
                i += 1;
                last_segment_type = LastSegmentType::WeatherReport;
            }

        } else if roll < 0.97 {
            // 4% chance: Show (rare, but always with introduction and show close)
            let next_show = show_pool.last().cloned();
            let context = CalloutContext {
                last_song: last_song.clone(),
                next_song: next_show,
            };
            segments.push_back(RadioSegment::ShowIntroduction(context));
            i += 1;

            if i < 64 {
                if let Some(show) = show_pool.pop() {
                    segments.push_back(RadioSegment::Show(show));
                    i += 1;

                    // Always add ShowClose after a show
                    if i < 64 {
                        let context = CalloutContext {
                            last_song: last_song.clone(),
                            next_song: peek_next_song(track_pool),
                        };
                        segments.push_back(RadioSegment::ShowClose(context));
                        i += 1;
                    }

                    last_segment_type = LastSegmentType::None;
                }
            }

        } else {
            // 3% chance: Random radio callout (only if last wasn't radio callout)
            if last_segment_type != LastSegmentType::RadioCallout {
                let context = CalloutContext {
                    last_song: last_song.clone(),
                    next_song: peek_next_song(track_pool),
                };
                segments.push_back(RadioSegment::RadioCallout(context));
                i += 1;
                last_segment_type = LastSegmentType::RadioCallout;
            }
        }
    }

    segments
}