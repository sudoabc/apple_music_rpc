use serde;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use regex::Regex;

#[derive(Deserialize, Clone)]
pub struct AppleApiResponse {
    results: Vec<AppleTrack>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AppleTrack {
    pub artwork_url100: Option<String>,
    pub track_view_url: Option<String>,
}

pub static SONG_CACHE: LazyLock<Mutex<HashMap<String, AppleTrack>>> = 
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub struct SongInfo {
    pub title: String,
    pub artist: String,
    pub length_ms: u32,
    
    pub is_playing: bool,
    pub position_ms: u32,
    
    pub cover_url: Option<String>, 
    pub song_url: Option<String>,
}

impl SongInfo {
    pub fn new(title: &str, artist: &str, length_ms: u32, is_playing: bool, position_ms: u32) -> Self {
        let re = Regex::new(r"\s+[\-—]\s+.*").unwrap();
        let clean_artist = re.replace_all(artist, "").trim().to_string();
        Self {
            title: title.to_string(),
            artist: clean_artist,
            length_ms,
            is_playing,
            position_ms,
            cover_url: None, 
            song_url: None,
        }
    }

    pub async fn fetch_api_data(&mut self) {
        let query = format!("{} {}", self.artist, self.title).replace(" ", "+");
        
        {
            let cache_lock = SONG_CACHE.lock().unwrap();
            if let Some(cached_track) = cache_lock.get(&query) {
                if let Some(art_work) = &cached_track.artwork_url100 {
                    self.cover_url = Some(art_work.replace("100x100bb", "512x512bb"));
                }

                self.song_url = cached_track.track_view_url.clone();
                return;
            }
        }

        let url = format!("https://itunes.apple.com/search?term={}&entity=song&limit=1", query);

        if let Ok(response) = reqwest::get(&url).await {
            if let Ok(json) = response.json::<AppleApiResponse>().await {
                if let Some(track) = json.results.first() {
                    if let Some(art_work) = &track.artwork_url100 {
                        self.cover_url = Some(art_work.replace("100x100bb", "512x512bb"));
                    }

                    self.song_url = track.track_view_url.clone();
                    {
                        let mut cache_lock = SONG_CACHE.lock().unwrap();
                        cache_lock.insert(query, track.clone());
                    }
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
pub async fn get_current_song() -> Option<SongInfo> {
    use tokio::process::Command;

    let script = r#"
        tell application "Music"
            set track_state to player state as string
            if track_state is "playing" or track_state is "paused" then
                set track_name to name of current track
                set track_artist to artist of current track
                set track_album to album of current track
                set track_duration to duration of current track
                set track_pos to player position
                return track_state & "|" & track_name & "|" & track_artist & "|" & track_album & "|" & track_duration & "|" & track_pos
            end if
        end tell
    "#;
    
    let output = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .await
        .ok()?;

    let result = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if result.is_empty() {
        return None;
    }

    let parts: Vec<&str> = result.split('|').collect();
    if parts.len() == 6 {
        let is_playing = parts[0] == "playing";
        let title = parts[1];
        let artist = parts[2];

        let duration_sec: f64 = parts[4].replace(',', ".").parse().unwrap_or(0.0);
        let pos_sec: f64 = parts[5].replace(',', ".").parse().unwrap_or(0.0);
        
        return Some(SongInfo::new(
            title, artist, 
            (duration_sec * 1000.0) as u32, 
            is_playing, 
            (pos_sec * 1000.0) as u32
        ));
    }
    None
}

#[cfg(target_os = "windows")]
pub async fn get_current_song() -> Option<SongInfo> {
    use windows::Media::Control::{
        GlobalSystemMediaTransportControlsSessionManager, 
        GlobalSystemMediaTransportControlsSessionPlaybackStatus
    };

    let manager_req = GlobalSystemMediaTransportControlsSessionManager::RequestAsync();
    let manager_async = match manager_req {
        Ok(req) => req,
        Err(e) => {
            println!("[DEBUG] Error at RequestAsync: {:?}", e);
            return None;
        }
    };

    let manager = match manager_async.await {
        Ok(m) => m,
        Err(e) => {
            println!("[DEBUG] Error at await manager: {:?}", e);
            return None;
        }
    };

    let sessions = match manager.GetSessions() {
        Ok(s) => s,
        Err(e) => {
            println!("[DEBUG] Error at GetSessions: {:?}", e);
            return None;
        }
    };

    let mut target_session = None;

    for session in sessions {
        if let Ok(app_id) = session.SourceAppUserModelId() {
            let app_id_str = app_id.to_string().to_lowercase();
            
            if app_id_str.contains("applemusicwin") {
                target_session = Some(session);
                break;
            }
        }
    }

    if let Some(session) = target_session {        
        let props = session.TryGetMediaPropertiesAsync().ok()?.await.ok()?;
        let title = props.Title().ok()?.to_string();
        
        if !title.is_empty() {
            let artist = props.Artist().ok()?.to_string();
        
            let info = session.GetPlaybackInfo().ok()?;
            let is_playing = info.PlaybackStatus().ok()? == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing;

            let timeline = session.GetTimelineProperties().ok()?;
            let length_ms = (timeline.EndTime().ok()?.Duration / 10_000) as u32;
            let position_ms = (timeline.Position().ok()?.Duration / 10_000) as u32;

            return Some(SongInfo::new(
                &title,
                &artist,
                length_ms,
                is_playing,
                position_ms,
            ));
        } else {
            println!("[DEBUG] Application found, but the song title is empty.");
        }
    }
    
    None
}