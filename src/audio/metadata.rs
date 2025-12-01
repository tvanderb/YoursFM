use serde::Deserialize;
use crate::YoursFMTrack;

#[derive(Debug, Deserialize)]
struct LastFmResponse {
    track: Option<LastFmTrack>,
}

#[derive(Debug, Deserialize)]
struct LastFmTrack {
    wiki: Option<LastFmWiki>,
}

#[derive(Debug, Deserialize)]
struct LastFmWiki {
    summary: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct MusicBrainzResponse {
    recordings: Option<Vec<MusicBrainzRecording>>,
}

#[derive(Debug, Deserialize)]
struct MusicBrainzRecording {
    #[serde(rename = "first-release-date")]
    first_release_date: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TrackMetadata {
    pub release_year: Option<u32>,
    pub wiki_summary: Option<String>,
    pub wiki_content: Option<String>,
}

pub struct MetadataFetcher {
    lastfm_api_key: String,
    musicbrainz_contact: String,
    client: reqwest::Client,
}

impl MetadataFetcher {
    pub fn new(lastfm_api_key: String, musicbrainz_contact: String) -> Self {
        Self {
            lastfm_api_key,
            musicbrainz_contact,
            client: reqwest::Client::new(),
        }
    }

    /// Fetch comprehensive metadata using hybrid approach:
    /// 1. Last.fm for wiki content (cultural context)
    /// 2. MusicBrainz for release year (factual data)
    pub async fn fetch_track_metadata(&self, track: &YoursFMTrack) -> TrackMetadata {
        let lastfm_data = self.fetch_lastfm_wiki(&track.artist, &track.name).await;
        let release_year = self.fetch_musicbrainz_year(&track.artist, &track.name).await;

        TrackMetadata {
            release_year,
            wiki_summary: lastfm_data.as_ref().and_then(|(s, _)| Some(s.clone())),
            wiki_content: lastfm_data.and_then(|(_, c)| Some(c)),
        }
    }

    /// Fetch Last.fm wiki content for cultural/historical context
    async fn fetch_lastfm_wiki(&self, artist: &str, track: &str) -> Option<(String, String)> {
        let url = format!(
            "http://ws.audioscrobbler.com/2.0/?method=track.getInfo&api_key={}&artist={}&track={}&format=json&autocorrect=1",
            self.lastfm_api_key,
            urlencoding::encode(artist),
            urlencoding::encode(track)
        );

        println!("[METADATA] Fetching Last.fm wiki for '{}' by {}", track, artist);

        let response = self.client
            .get(&url)
            .send()
            .await
            .ok()?;

        if !response.status().is_success() {
            eprintln!("[METADATA] Last.fm API error: {}", response.status());
            return None;
        }

        let data: LastFmResponse = response.json().await.ok()?;
        let wiki = data.track?.wiki?;

        // Clean up the summary (Last.fm adds "Read more on Last.fm" links)
        let summary = wiki.summary
            .split("<a href")
            .next()
            .unwrap_or(&wiki.summary)
            .trim()
            .to_string();

        let content = wiki.content
            .split("<a href")
            .next()
            .unwrap_or(&wiki.content)
            .trim()
            .to_string();

        println!("[METADATA] ✓ Found Last.fm wiki ({} chars)", summary.len());
        Some((summary, content))
    }

    /// Fetch MusicBrainz release year for factual metadata
    async fn fetch_musicbrainz_year(&self, artist: &str, track: &str) -> Option<u32> {
        let query = format!("recording:\"{}\" AND artist:\"{}\"", track, artist);
        let url = format!(
            "https://musicbrainz.org/ws/2/recording/?query={}&fmt=json&limit=1",
            urlencoding::encode(&query)
        );

        println!("[METADATA] Fetching MusicBrainz year for '{}' by {}", track, artist);

        let response = self.client
            .get(&url)
            .header("User-Agent", format!("YoursFM/0.1.0 ( {} )", self.musicbrainz_contact))
            .send()
            .await
            .ok()?;

        if !response.status().is_success() {
            eprintln!("[METADATA] MusicBrainz API error: {}", response.status());
            return None;
        }

        // MusicBrainz rate limiting - be polite
        tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;

        let data: MusicBrainzResponse = response.json().await.ok()?;
        let recording = data.recordings?.into_iter().next()?;
        let date_str = recording.first_release_date?;

        // Parse year from YYYY-MM-DD format
        let year = date_str.split('-').next()?.parse::<u32>().ok()?;

        println!("[METADATA] ✓ Found release year: {}", year);
        Some(year)
    }
}
