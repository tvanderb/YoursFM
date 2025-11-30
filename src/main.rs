mod spotify;

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

#[derive(Debug)]
enum RadioSegment {
    NextSongCallout,
    LastSongCallout,
    RadioCallout,
    ShowIntroduction,
    NextSongCalloutDetailed,
    NextSongCalloutHistory,
    NewsReport,
    WeatherReport,
    Song(YoursFMTrack),
    Show(YoursFMTrack),
    ShowClose,
}

#[derive(Debug, Serialize, Deserialize)]
struct StationLocation {
    name: String,
    airport: String
}

#[derive(Debug, Serialize, Deserialize)]
struct StationConfiguration {
    code: String,
    name: String,
    frequency: String,
    channel: String,
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
struct Configuration {
    spotify: SpotifyConfiguration,
    station: StationConfiguration,
    database: DatabaseConfiguration,
}

#[derive(Debug)]
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
        .get_matches();

    // Parse verbosity level
    let verbosity: LogVerbosity = match command_argument_matches.get_one::<u8>("verbose").expect("Verbosity is defaulted.") {
        0 => LogVerbosity::OFF,
        1 => LogVerbosity::MINIMAL,
        2 => LogVerbosity::INCREASED,
        3 => LogVerbosity::FULL,
        _ => LogVerbosity::OFF
    };

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

    let (mut songs, mut shows) = refresh_tracks(&config, sqlite_connection_pool).await;

    // Shuffle the track and show pools for variety
    let mut rng = rand::thread_rng();
    songs.shuffle(&mut rng);
    shows.shuffle(&mut rng);

    loop {
        // Populate the segment list
        let segments = populate_radio_segments(&mut songs, &mut shows);

        // Generate the show by traversing segments
        for segment in segments.iter() {
            println!("{:#?}", segment);
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    }
}

fn populate_radio_segments(
    track_pool: &mut Vec<YoursFMTrack>,
    show_pool: &mut Vec<YoursFMTrack>,
) -> LinkedList<RadioSegment> {
    let mut segments: LinkedList<RadioSegment> = LinkedList::new();
    let mut rng = rand::thread_rng();

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
                let callout_type: f32 = rng.r#gen();
                if callout_type < 0.6 {
                    segments.push_back(RadioSegment::NextSongCallout);
                } else if callout_type < 0.8 {
                    segments.push_back(RadioSegment::NextSongCalloutDetailed);
                } else {
                    segments.push_back(RadioSegment::NextSongCalloutHistory);
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
                    segments.push_back(RadioSegment::Song(track));
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
                    segments.push_back(RadioSegment::RadioCallout);
                    i += 1;
                    last_segment_type = LastSegmentType::RadioCallout;
                }
            }

            // Less common: last_song_callout after songs (25% chance)
            if rng.r#gen::<f32>() < 0.25 && i < 64 {
                segments.push_back(RadioSegment::LastSongCallout);
                i += 1;
            }

        } else if roll < 0.85 {
            // 10% chance: News report (only if last wasn't news)
            if last_segment_type != LastSegmentType::NewsReport {
                segments.push_back(RadioSegment::NewsReport);
                i += 1;
                last_segment_type = LastSegmentType::NewsReport;
            }
            // If duplicate, skip and let next iteration pick something else

        } else if roll < 0.93 {
            // 8% chance: Weather report (only if last wasn't weather)
            if last_segment_type != LastSegmentType::WeatherReport {
                segments.push_back(RadioSegment::WeatherReport);
                i += 1;
                last_segment_type = LastSegmentType::WeatherReport;
            }

        } else if roll < 0.97 {
            // 4% chance: Show (rare, but always with introduction)
            segments.push_back(RadioSegment::ShowIntroduction);
            i += 1;

            if i < 64 {
                if let Some(show) = show_pool.pop() {
                    segments.push_back(RadioSegment::Show(show));
                    i += 1;

                    // Always add ShowClose after a show
                    if i < 64 {
                        segments.push_back(RadioSegment::ShowClose);
                        i += 1;
                    }

                    last_segment_type = LastSegmentType::None;
                }
            }

        } else {
            // 3% chance: Random radio callout (only if last wasn't radio callout)
            if last_segment_type != LastSegmentType::RadioCallout {
                segments.push_back(RadioSegment::RadioCallout);
                i += 1;
                last_segment_type = LastSegmentType::RadioCallout;
            }
        }
    }

    segments
}