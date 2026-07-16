use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use tokio::time::{sleep, Duration};
use regex::Regex;

#[cfg(target_os = "macos")]
use std::sync::{
    atomic::{AtomicBool},
    Arc, OnceLock,
};

#[derive(Deserialize, Clone)]
pub struct AppleApiResponse {
    results: Vec<AppleTrack>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AppleTrack {
    pub track_name: String, 
    pub artist_name: String,
    pub artwork_url100: Option<String>,
    pub track_view_url: Option<String>,
}

static ARTIST_CLEAN_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+[\-—]\s+.*").unwrap());
const MAX_CACHE_ENTRIES: usize = 128;
// Thread-safe global cache to prevent iTunes API rate-limiting
pub static SONG_CACHE: LazyLock<Mutex<HashMap<String, AppleTrack>>> = 
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Clone, PartialEq)]
pub struct SongInfo {
    pub title: String,
    pub artist: String,
    pub length_ms: u32,
    
    pub is_playing: bool,
    pub position_ms: u32,
    
    pub song_id: Option<String>,
    pub cover_url: Option<String>, 
    pub song_url: Option<String>,
}

impl SongInfo {
    pub fn new(
        title: &str, 
        artist: &str, 
        length_ms: u32, 
        is_playing: bool, 
        position_ms: u32,
        song_id: Option<String>
    ) -> Self {
        // Remove unwanted characters from the artist name (e.g., Apple-specific dashes)
        let clean_artist = ARTIST_CLEAN_RE
            .replace_all(artist, "")
            .trim()
            .to_string();

        Self {
            title: title.to_string(),
            artist: clean_artist,
            length_ms,
            is_playing,
            position_ms,
            song_id,
            cover_url: None, 
            song_url: None,
        }
    }

    async fn do_request(&self, url: String) -> Option<Vec<AppleTrack>> {
        if let Ok(response) = reqwest::get(&url).await 
            && let Ok(json) = response.json::<AppleApiResponse>().await
        {
            Some(json.results)
        } else {
            None
        }
    }

    fn set_artwork(&mut self, track: &AppleTrack) {
        if let Some(art_work) = &track.artwork_url100 {
            self.cover_url = Some(art_work.replace("100x100bb", "512x512bb"));
        }
        self.song_url = track.track_view_url.clone();
    }

    fn set_cache(&self, query: String, track: AppleTrack) {
        let mut cache_lock = SONG_CACHE.lock()
            .unwrap_or_else(|e| e.into_inner());

        if cache_lock.len() >= MAX_CACHE_ENTRIES {
            cache_lock.clear();
        }

        cache_lock.insert(query, track);
    }

    pub async fn fetch_api_data(&mut self) {
        let query = format!("{} {}", self.title, self.artist);
        
        // Check if we already fetched the image and song url
        {
            let cache_lock = SONG_CACHE
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if let Some(cached_track) = cache_lock.get(&query) {
                self.set_artwork(cached_track);
                return;
            }
        }

        if let Some(song_id) = self.song_id.clone() {
            // Fetching artwork and song url precisely using the song_id (MacOS case)
            let url = format!(
                "https://itunes.apple.com/lookup?id={}",
                song_id
            );
            let results = self.do_request(url).await;

            if let Some(track) = results
                .and_then(|tracks| tracks.into_iter().next()) 
            {
                self.set_artwork(&track);
                self.set_cache(song_id.clone(), track); 
            }
        } else {
            // No Song ID available. Fetching using song name and artist
            let title_lower = self.title.to_lowercase().trim().to_string();
            let artist_lower = self.artist.to_lowercase().trim().to_string();

            let url = format!(
                "https://itunes.apple.com/search?term={}&entity=song&limit=5", 
                urlencoding::encode(&query)
            );
            let results = self.do_request(url).await;

            let mut found_track = results.unwrap_or_default().into_iter().find(|res| {
                let api_title = res.track_name.to_lowercase();
                (api_title == title_lower || api_title.contains(&title_lower) || title_lower.contains(&api_title))
                && res.artist_name.to_lowercase().contains(&artist_lower)
            });

            if found_track.is_none() {
                let url_brute = format!(
                    "https://itunes.apple.com/search?term={}&entity=song&limit=200", 
                    urlencoding::encode(&self.artist)
                );
                let results_brute = self.do_request(url_brute).await;

                found_track = results_brute.unwrap_or_default().into_iter().find(|res| {
                    let api_title = res.track_name.to_lowercase();
                    (api_title.contains(&title_lower) || title_lower.contains(&api_title))
                    && res.artist_name.to_lowercase().contains(&artist_lower)
                });
            }   
            if let Some(track) = found_track {
                self.set_artwork(&track);
                self.set_cache(query, track);
            }
        }
    }
}

// MacOS: FFI Bridge (Foreign Function Interface)

#[cfg(target_os = "macos")]
struct MacManager;
#[cfg(target_os = "macos")]
unsafe impl Send for MacManager {}
#[cfg(target_os = "macos")]
unsafe impl Sync for MacManager {}

#[cfg(target_os = "macos")]
#[allow(dead_code)]
struct MacListenerState {
    ping_tx: tokio::sync::mpsc::UnboundedSender<()>,
    running: AtomicBool,
}

#[cfg(target_os = "macos")]
#[allow(dead_code)]
static MAC_LISTENER_STATE: OnceLock<Arc<MacListenerState>> = OnceLock::new();

#[cfg(target_os = "macos")]
mod mac_ffi {
    use super::{MAC_LISTENER_STATE};
    use std::ffi::c_void;
    use std::sync::atomic::Ordering;

    // Bind to native Apple API functions (CoreFoundation)
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        pub fn CFNotificationCenterGetDistributedCenter() -> *mut c_void;
        pub fn CFNotificationCenterAddObserver(
            center: *mut c_void, 
            observer: *mut c_void, 
            callback: Option<
                extern "C" fn(
                    *mut c_void, 
                    *const c_void, 
                    *const c_void, 
                    *const c_void, 
                    *const c_void
                ),
            >, 
            name: *const c_void, 
            object: *const c_void, 
            suspensionBehavior: isize
        );  
        pub fn CFStringCreateWithCString(
            alloc: *mut c_void, 
            cStr: *const i8, 
            encoding: u32
        ) -> *mut c_void;
        pub fn CFRelease(cf: *mut c_void);
        pub fn CFRunLoopRun();
    }
    // Callback triggered by the OS when the song changes
    pub extern "C" fn notification_callback(
        _center: *mut c_void,
        observer: *const c_void,
        _name: *const c_void,
        _object: *const c_void,
        _user_info: *const c_void,
    ) {
        if !observer.is_null() 
            && let Some(state) = MAC_LISTENER_STATE.get() 
            && state.running.load(Ordering::SeqCst)
        {
            let _ = state.ping_tx.send(());
        }
    }
}

#[cfg(target_os = "macos")]
impl MacManager {
    unsafe fn run_observer(ptr: *mut std::ffi::c_void) {
        unsafe {
            let center = mac_ffi::CFNotificationCenterGetDistributedCenter();
            let name = mac_ffi::CFStringCreateWithCString(
                std::ptr::null_mut(),
                c"com.apple.iTunes.playerInfo".as_ptr(),
                0x08000100,
            );

            // Subscribe to the Apple Music player then keeping the thread alive to receive events
            mac_ffi::CFNotificationCenterAddObserver(
                center, 
                ptr, 
                Some(mac_ffi::notification_callback), 
                name, 
                std::ptr::null(), 
                2
            );
            mac_ffi::CFRelease(name);
            mac_ffi::CFRunLoopRun();
        }
    }
}

#[cfg(target_os = "macos")]
pub async fn start_listener(tx: tokio::sync::mpsc::Sender<Option<SongInfo>>) {
    use tokio::process::Command;
    use tokio::sync::mpsc;

    // Internal channel to wake up Tokio when notified by C
    let (ping_tx, mut ping_rx) = mpsc::unbounded_channel::<()>();

    let state = Arc::new(MacListenerState {
        ping_tx: ping_tx.clone(),
        running: AtomicBool::new(true),
    });

    let _ = MAC_LISTENER_STATE.set(state);

    // Run the C loop in a dedicated thread to prevent blocking the async runtime
    std::thread::spawn(|| {
        unsafe {
            MacManager::run_observer(std::ptr::null_mut());
        }
    });

    // Force an initial read on app startup
    let _ = ping_tx.send(());

    let script = r#"
        tell application "Music"
            set track_state to player state as string
            if track_state is "playing" or track_state is "paused" then
                set track_name to name of current track
                set track_artist to artist of current track
                set track_duration to duration of current track
                set track_pos to player position
                
                try
                    set song_id to store id of current track as string
                on error
                    set song_id to "None"
                end try

                return track_state & "|" & track_name & "|" & track_artist & "|" & track_duration & "|" & track_pos & "|" & song_id
            end if
        end tell
    "#;

    // This executes strictly when receiving a PING
    while (ping_rx.recv().await).is_some() {
        sleep(Duration::from_secs(1)).await;

        while ping_rx.try_recv().is_ok() {}

        if let Ok(output) = Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .await 
        {
            let result = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !result.is_empty() {
                let parts: Vec<&str> = result.split('|').collect();
                if parts.len() == 6 {
                    let is_playing = parts[0] == "playing";
                    let duration_sec: f64 = parts[3].replace(",", ".").parse().unwrap_or(0.0);
                    let position_sec: f64 = parts[4].replace(",", ".").parse().unwrap_or(0.0);
                    let song_id: Option<String> = if parts[5] == "None" {
                        None
                    } else {
                        Some(parts[5].to_string())
                    };

                    let song = SongInfo::new(
                        parts[1], 
                        parts[2],
                        (duration_sec * 1000.0) as u32,
                        is_playing,
                        (position_sec * 1000.0) as u32,
                        song_id
                    );

                    let _ = tx.send(Some(song)).await.ok();
                    continue;
                }
            }
        }
        let _ = tx.send(None).await.ok();
    }
}

// WINDOWS: SMTC Event-Driven
#[cfg(target_os = "windows")]
pub async fn start_listener(tx: tokio::sync::mpsc::Sender<Option<SongInfo>>) {
    use windows::Media::Control::{
        GlobalSystemMediaTransportControlsSession,
        GlobalSystemMediaTransportControlsSessionManager,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus,
    };
    use windows::Foundation::TypedEventHandler;

    // Internal channel to bridge Windows events with Tokio tasks
    let (ping_tx, mut ping_rx) = tokio::sync::mpsc::unbounded_channel::<()>();

    let manager = match GlobalSystemMediaTransportControlsSessionManager::RequestAsync() {
        Ok(req) => match req.await {
            Ok(m) => m,
            Err(_) => return,
        },
        Err(_) => return,
    };

    // App Open/Closure event
    let ping_clone = ping_tx.clone();
    let _ = manager.SessionsChanged(&TypedEventHandler::new(move |_, _| {
        let _ = ping_clone.send(());
        Ok(())
    }));

    let mut is_subscribed = false;
    let _ = ping_tx.send(());

    while (ping_rx.recv().await).is_some() {
        sleep(Duration::from_secs(1)).await;

        while ping_rx.try_recv().is_ok() {}

        let sessions = match manager.GetSessions() {
            Ok(s) => s,
            Err(_) => return,
        };

        let mut target_session: Option<GlobalSystemMediaTransportControlsSession> = None;

        // Search for the specific Apple Music session (ignore Spotify/YouTube)
        for session in sessions {
            if let Ok(app_id) = session.SourceAppUserModelId() 
                && app_id.to_string().to_lowercase().contains("applemusicwin") 
            {
                target_session = Some(session);
                break;
            }
        }

        if let Some(session) = target_session {
            if !is_subscribed {
                is_subscribed = true;
                
                // Song changed event
                let p1 = ping_tx.clone();
                let _ = session.MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
                    let _ = p1.send(());
                    Ok(())
                }));
                
                // Pause/Play event
                let p2 = ping_tx.clone();
                let _ = session.PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
                    let _ = p2.send(());
                    Ok(())
                }));
                
                // Seek / Timeline event
                let p3 = ping_tx.clone();
                    let _ = session.TimelinePropertiesChanged(&TypedEventHandler::new(move |_, _| {
                    let _ = p3.send(());
                    Ok(())
                }));
            }

            // Extract the actual details (Title, Artist, Album, Time)
            let song_info = async {
                let props = session.TryGetMediaPropertiesAsync().ok()?.await.ok()?;
                let title = props.Title().ok()?.to_string();
                if title.is_empty() {
                    return None;
                }

                let artist = props.Artist().ok()?.to_string();
                let info = session.GetPlaybackInfo().ok()?;
                let is_playing = info.PlaybackStatus().ok()? 
                    == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing;
                
                let mut length_ms = 0;
                let mut position_ms = 0;

                for _ in 0..10 {
                    if let Ok(timeline) = session.GetTimelineProperties() {
                        let current_length = (timeline.EndTime().ok()?.Duration / 10_000) as u32;

                        if current_length > 0 {
                            length_ms = current_length;
                            position_ms = (timeline.Position().ok()?.Duration / 10_000) as u32;
                            break;
                        }
                    }

                    sleep(Duration::from_millis(500)).await;
                }

                Some(SongInfo::new(
                    &title, 
                    &artist, 
                    length_ms, 
                    is_playing, 
                    position_ms,
                    None // Can't fetch apple music song id on windows or i didn't figure it out at least
                ))
            }
            .await;

            if tx.send(song_info).await.is_err() { 
                break; 
            }
        } else {
            is_subscribed = false;
            if tx.send(None).await.is_err() { 
                break; 
            }
        }
    }
}