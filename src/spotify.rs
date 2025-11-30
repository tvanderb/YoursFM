use serde::{Deserialize, Serialize};
use crate::{generate_track_id, YoursFMTrack};

#[derive(Debug, Serialize, Deserialize)]
struct SpotifyTokenRequestResponse {
    access_token: String,
    token_type: String,
    expires_in: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct SpotifyTrackAlbum {
    name: String,
    release_date: String
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct SpotifyTrackArtist {
    name: String
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct SpotifyTrack {
    album: SpotifyTrackAlbum,
    name: String,
    artists: Vec<SpotifyTrackArtist>
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct SpotifyPlaylistTrackItem {
    track: SpotifyTrack
}

#[derive(Debug, Serialize, Deserialize)]
struct SpotifyPlaylistTracksResponse {
    items: Vec<SpotifyPlaylistTrackItem>,
    next: Option<String>
}

async fn get_spotify_tracks_from_playlist(spotify_client: reqwest::Client, url: String, existing: Option<Vec<SpotifyPlaylistTrackItem>>) -> Vec<SpotifyPlaylistTrackItem> {
    let response = match spotify_client.get(url)
        .send()
        .await {
        Ok(response) => response,
        Err(error) => panic!("Spotify API playlist tracks request failed. Error: {:#?}", error)
    };

    let text = match response.text().await {
        Ok(text) => text,
        Err(error) => panic!("Spotify API playlist tracks request failed. Error: {:#?}", error)
    };

    let mut data: SpotifyPlaylistTracksResponse = match serde_json::from_str(text.as_str()) {
        Ok(data) => data,
        Err(error) => panic!("Spotify API playlist tracks deserialization failed. Error: {:#?}", error)
    };

    let mut tracks: Vec<SpotifyPlaylistTrackItem> = match existing {
        Some(existing) => existing,
        None => vec![]
    };

    tracks.append(&mut data.items);

    let mut next_track: Vec<SpotifyPlaylistTrackItem> = match data.next {
        None => vec![],
        Some(next) => (*Box::pin(get_spotify_tracks_from_playlist(spotify_client, next, Some(tracks.clone()))).await).to_owned()
    };

    tracks.append(&mut next_track);

    tracks
}

async fn get_all_spotify_tracks_from_playlist(client_id: String, client_secret: String, playlist_id: String) -> Vec<SpotifyPlaylistTrackItem> {
    let spotify_client = create_spotify_client(client_id, client_secret).await;

    let base_url = format!("https://api.spotify.com/v1/playlists/{}/tracks?limit=50", playlist_id);
    get_spotify_tracks_from_playlist(spotify_client, base_url, None).await
}

async fn create_spotify_client(client_id: String, client_secret: String) -> reqwest::Client {
    let access_token = {
        let client = reqwest::Client::new();
        let response = match client.post("https://accounts.spotify.com/api/token")
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", client_id.as_str()),
                ("client_secret", client_secret.as_str()),
            ])
            .send()
            .await {
            Ok(response) => response,
            Err(error) => panic!("Spotify API token request failed. Error: {:#?}", error)
        };

        let text = match response.text().await {
            Ok(text) => text,
            Err(error) => panic!("Spotify API token request failed. Error: {:#?}", error)
        };

        let data: SpotifyTokenRequestResponse = match serde_json::from_str(text.as_str()) {
            Ok(data) => data,
            Err(error) => panic!("Spotify API token request deserialization error. Error: {:#?}", error)
        };

        format!("Bearer {}", data.access_token)
    };

    let mut headers = reqwest::header::HeaderMap::new();

    headers.insert(reqwest::header::AUTHORIZATION, access_token.parse().unwrap());

    match reqwest::Client::builder()
        .default_headers(headers)
        .build() {
        Ok(client) => client,
        Err(error) => panic!("Spotify API client builder error: {:#?}", error)
    }
}
pub async fn get_all_yoursfm_tracks_from_spotify_playlist(client_id: String, client_secret: String, playlist_id: String) -> Vec<YoursFMTrack> {
    let spotify_tracks = get_all_spotify_tracks_from_playlist(client_id, client_secret, playlist_id).await;

    let mut yoursfm_tracks: Vec<YoursFMTrack> = vec![];

    for mut x in spotify_tracks {
        let mut artists: String = x.track.artists.pop().unwrap().name;

        for artist in x.track.artists {
            artists = format!("{}, {}", artists, artist.name);
        }

        yoursfm_tracks.push(YoursFMTrack {
            id: generate_track_id(x.track.name.clone(), artists.clone(), x.track.album.name.clone(), x.track.album.release_date.clone()),
            name: x.track.name,
            album_release_date: x.track.album.release_date,
            artist: artists,
            album_name: x.track.album.name,
        });
    }

    yoursfm_tracks
}