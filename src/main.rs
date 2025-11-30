mod spotify;

use reqwest::{Error, Response};
use serde::{Deserialize, Serialize};

const DEFAULT_CONFIGURATION_FILEPATH: &str = "./config.yaml";

#[derive(Debug)]
enum LogVerbosity {
    OFF,
    MINIMAL,
    INCREASED,
    FULL
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
struct Configuration {
    spotify: SpotifyConfiguration,
    station: StationConfiguration,
}

#[derive(Debug)]
struct YoursFMTrack {
    name: String,
    artist: String,
    album_name: String,
    album_release_date: String
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

    let songs = spotify::get_all_yoursfm_tracks_from_spotify_playlist(config.spotify.client_id.clone(), config.spotify.client_secret.clone(), config.spotify.songs_playlist_id).await;

    for x in songs {
        println!("{:#?}", x)
    }

    let shows = spotify::get_all_yoursfm_tracks_from_spotify_playlist(config.spotify.client_id, config.spotify.client_secret, config.spotify.shows_playlist_id).await;
    
    // TODO: Procedurally generate program and callouts using available shows and songs
}