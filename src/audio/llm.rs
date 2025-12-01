use serde::{Deserialize, Serialize};
use std::sync::Arc;
use crate::{RadioSegment, CalloutContext, StationConfiguration};
use super::metadata::MetadataFetcher;

#[derive(Debug, Serialize)]
struct OpenRouterRequest {
    model: String,
    messages: Vec<ChatMessage>,
}

#[derive(Debug, Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct OpenRouterResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: Message,
}

#[derive(Debug, Deserialize)]
struct Message {
    content: String,
}

pub struct LLMGenerator {
    api_key: String,
    model: String,
    client: reqwest::Client,
    metadata_fetcher: Arc<MetadataFetcher>,
}

impl LLMGenerator {
    pub fn new(api_key: String, model: String, metadata_fetcher: Arc<MetadataFetcher>) -> Self {
        Self {
            api_key,
            model,
            client: reqwest::Client::new(),
            metadata_fetcher,
        }
    }

    pub async fn generate_callout_text(
        &self,
        segment: &RadioSegment,
        station: &StationConfiguration,
    ) -> Result<String, String> {
        let prompt = self.build_prompt(segment, station).await?;

        let request = OpenRouterRequest {
            model: self.model.clone(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: prompt,
            }],
        };

        let response = self.client
            .post("https://openrouter.ai/api/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| format!("OpenRouter request failed: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("OpenRouter API error {}: {}", status, body));
        }

        let data: OpenRouterResponse = response.json().await
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        let text = data.choices
            .first()
            .and_then(|c| Some(c.message.content.clone()))
            .ok_or("No response from LLM")?;

        Ok(text.trim().to_string())
    }

    async fn build_prompt(&self, segment: &RadioSegment, station: &StationConfiguration) -> Result<String, String> {
        let station_id = station.identifier();
        let station_full = station.full_name();

        match segment {
            RadioSegment::NextSongCallout(ctx) => {
                Ok(self.next_song_prompt(ctx, &station_id, &station_full))
            }
            RadioSegment::LastSongCallout(ctx) => {
                Ok(self.last_song_prompt(ctx, &station_id, &station_full))
            }
            RadioSegment::RadioCallout(ctx) => {
                Ok(self.radio_callout_prompt(ctx, &station_id, &station_full))
            }
            RadioSegment::NextSongCalloutDetailed(ctx) => {
                self.next_song_detailed_prompt(ctx, &station_id, &station_full).await
            }
            RadioSegment::NextSongCalloutHistory(ctx) => {
                self.next_song_history_prompt(ctx, &station_id, &station_full).await
            }
            RadioSegment::NewsReport(_) => {
                Ok(format!("You're a radio DJ on {}. Give a brief 6-8 word transition to news. Be succinct. Avoid filler like 'and now', 'coming up' - be direct.", station_full))
            }
            RadioSegment::WeatherReport(_) => {
                Ok(format!("You're a radio DJ on {}. Give a brief 6-8 word transition to weather. Be succinct. Avoid filler like 'and now', 'coming up' - be direct.", station_full))
            }
            RadioSegment::ShowIntroduction(ctx) => {
                Ok(self.show_intro_prompt(ctx, &station_id, &station_full))
            }
            RadioSegment::ShowClose(_) => {
                Ok(format!("You're a radio DJ on {}. Thank listeners for that show in 6-8 words. Be succinct and warm. Avoid filler - be direct.", station_full))
            }
            _ => {
                Ok(format!("You're a radio DJ on {}. Give a 4-6 word station identification. Be succinct.", station_id))
            }
        }
    }

    fn next_song_prompt(&self, ctx: &CalloutContext, station_id: &str, station_full: &str) -> String {
        if let Some(next) = &ctx.next_song {
            format!(
                "You're a DJ on {}. Briefly introduce the next song: '{}' by {}. \
                Keep it under 8 words, succinct and direct. Just the track and artist - NO additional details. \
                Avoid unnecessary filler like 'coming up', 'next up', 'now' - be more succinct. \
                Good examples: 'Next, CCR - Travellin' Band' or 'CCR, Travellin' Band'. Less is more. \
                IMPORTANT: Use the EXACT song title '{}' - do not shorten or change it. \
                You may abbreviate artist names ONLY if commonly done (e.g., 'CCR' for Creedence Clearwater Revival).",
                station_full, next.name, next.artist, next.name
            )
        } else {
            format!("You're a DJ on {}. Give a brief 10 word station callout.", station_id)
        }
    }

    fn last_song_prompt(&self, ctx: &CalloutContext, station_id: &str, _station_full: &str) -> String {
        if let Some(last) = &ctx.last_song {
            format!(
                "You're a DJ on {}. Mention the song that just played: '{}' by {}. \
                Keep it under 10 words, succinct and direct. Focus ONLY on the last song. \
                Avoid unnecessary filler like 'just played', 'thanks', 'enjoy' - be more succinct. \
                Good examples: 'That was The Doors Light My Fire' or 'The Doors, Light My Fire'. Less is more. \
                IMPORTANT: Use the EXACT song title '{}' - do not shorten or change it. \
                You may abbreviate artist names ONLY if commonly done (e.g., 'CCR' for Creedence Clearwater Revival).",
                station_id, last.name, last.artist, last.name
            )
        } else {
            format!("You're a DJ on {}. Give a brief 10 word callout.", station_id)
        }
    }

    fn radio_callout_prompt(&self, ctx: &CalloutContext, _station_id: &str, station_full: &str) -> String {
        let context_info = match (&ctx.last_song, &ctx.next_song) {
            (Some(last), Some(next)) => {
                format!("Last song: '{}'. Next song: '{}'.", last.name, next.name)
            }
            (Some(last), None) => format!("Last song: '{}'.", last.name),
            (None, Some(next)) => format!("Next song: '{}'.", next.name),
            _ => String::new(),
        };

        format!(
            "You're a DJ on {}. {} Give a succinct station identification callout. \
            Keep it under 12 words. Avoid unnecessary filler like 'just played', 'coming up', 'thanks' - be direct and succinct. \
            Good example with both songs: 'That was The Doors Light My Fire, Next up CCR' or 'The Doors, Light My Fire, next we've got CCR - Travellin' Band'. \
            Less is more.",
            station_full, context_info
        )
    }

    async fn next_song_detailed_prompt(&self, ctx: &CalloutContext, _station_id: &str, station_full: &str) -> Result<String, String> {
        if let Some(next) = &ctx.next_song {
            let metadata = self.metadata_fetcher.fetch_track_metadata(next).await;

            // Build context from metadata - REQUIRE at least one piece of factual data
            let context = match (metadata.release_year, metadata.wiki_summary) {
                (Some(year), Some(wiki)) => {
                    format!("Release year: {}. Context: {}", year, wiki)
                }
                (Some(year), None) => {
                    format!("Release year: {}", year)
                }
                (None, Some(wiki)) => {
                    format!("Context: {}", wiki)
                }
                (None, None) => {
                    return Err(format!("No metadata available for '{}' by {} - cannot generate detailed callout without factual context", next.name, next.artist));
                }
            };

            Ok(format!(
                "You're a DJ on {}. Introduce the next song '{}' by {} with ONE brief interesting tidbit. \
                Keep it under 18 words, succinct and informative. Avoid filler like 'coming up', 'next up' - be direct. \
                IMPORTANT: Use the EXACT song title '{}' - do not change it. \
                You may abbreviate artist names ONLY if commonly done. \
                CRITICAL: Base your tidbit ONLY on the following factual information - DO NOT use any other knowledge: \
                {}",
                station_full, next.name, next.artist, next.name, context
            ))
        } else {
            Ok(format!("You're a DJ on {}. Give a 10 word callout.", station_full))
        }
    }

    async fn next_song_history_prompt(&self, ctx: &CalloutContext, _station_id: &str, station_full: &str) -> Result<String, String> {
        if let Some(next) = &ctx.next_song {
            let metadata = self.metadata_fetcher.fetch_track_metadata(next).await;

            // REQUIRE wiki content for history - we need substantial historical context
            let historical_context = match (metadata.release_year, metadata.wiki_content.or(metadata.wiki_summary)) {
                (year, Some(wiki)) => {
                    let year_info = year.map(|y| format!("Release year: {}. ", y)).unwrap_or_default();
                    format!("{}Historical context: {}", year_info, wiki)
                }
                (Some(year), None) => {
                    // If we only have year and no wiki, we can't tell a proper history story
                    return Err(format!("Insufficient historical context for '{}' by {} - only have release year ({}) but no cultural/historical information", next.name, next.artist, year));
                }
                (None, None) => {
                    return Err(format!("No historical metadata available for '{}' by {} - cannot generate history callout without factual context", next.name, next.artist));
                }
            };

            Ok(format!(
                "You're a DJ on {}. Introduce the next song '{}' by {} and share a bit about its significant history or cultural impact. \
                Keep it under 22 words, succinct and informative. Tell a brief story about the track's legacy. Avoid filler - be direct. \
                IMPORTANT: Use the EXACT song title '{}' - do not change it. \
                You may abbreviate artist names ONLY if commonly done. \
                CRITICAL: Base your story ONLY on the following factual historical information - DO NOT use any other knowledge or make up facts: \
                {}",
                station_full, next.name, next.artist, next.name, historical_context
            ))
        } else {
            Ok(format!("You're a DJ on {}. Give a 10 word callout.", station_full))
        }
    }

    fn show_intro_prompt(&self, ctx: &CalloutContext, _station_id: &str, station_full: &str) -> String {
        if let Some(show) = &ctx.next_song {
            format!(
                "You're a DJ on {}. Introduce special show: '{}'. \
                Build anticipation in 12-15 words, exciting and succinct. Avoid filler - be direct.",
                station_full, show.name
            )
        } else {
            format!("You're a DJ on {}. Introduce an upcoming special show in 10-12 words. Be succinct.", station_full)
        }
    }
}
