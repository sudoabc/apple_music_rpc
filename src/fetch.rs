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
    
    pub cover_url: Option<String>, 
    pub song_url: Option<String>,
}

impl SongInfo {
    pub fn new(
        title: &str, 
        artist: &str, 
        length_ms: u32, 
        is_playing: bool, 
        position_ms: u32
    ) -> Self {
        // Remove unwanted characters from the artist name (e.g., Apple-specific dashes)
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
        let query = format!("{} {}", self.title, self.artist).replace(" ", "+");
        
        // Check if we already fetched the image and song url
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

        let url = format!(
            "https://itunes.apple.com/search?term={}&entity=song&limit=1", 
            query
        );

        if let Ok(response) = reqwest::get(&url).await 
            && let Ok(json) = response.json::<AppleApiResponse>().await
            && let Some(track) = json.results.first().cloned() 
        {
            if let Some(art_work) = &track.artwork_url100 {
                self.cover_url = Some(art_work.replace("100x100bb", "512x512bb"));
            }
            self.song_url = track.track_view_url.clone();
            
            {
                let mut cache_lock = SONG_CACHE.lock().unwrap();
                cache_lock.insert(query, track);
            }
        }
    }
}

// MacOS: FFI Bridge (Foreign Function Interface)

#[allow(dead_code)]
pub struct SendWrapper<T>(pub T);
unsafe impl<T> Send for SendWrapper<T> {}
unsafe impl<T> Sync for SendWrapper<T> {}

#[allow(dead_code)]
struct SendPtr(*mut std::ffi::c_void);
unsafe impl Send for SendPtr {}
unsafe impl Sync for SendPtr {}

#[cfg(target_os = "macos")]
mod mac_ffi {
    use std::ffi::c_void;
    use tokio::sync::mpsc::UnboundedSender;
    use super::SendWrapper;

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
        if !observer.is_null() {
            // Retrieve the Tokio channel from the raw pointer and send a PING
            let wrapper = observer as *const SendWrapper<UnboundedSender<()>>;
            unsafe {
                let tx = &(*wrapper).0;
                let _ = tx.send(());
            }
        }
    }
}

#[cfg(target_os = "macos")]
struct MacManager;
#[cfg(target_os = "macos")]
unsafe impl Send for MacManager {}
#[cfg(target_os = "macos")]
unsafe impl Sync for MacManager {}

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
            mac_ffi::CFRunLoopRun();
        }
    }

    // Wrapper function that accepts SendPtr to ensure thread safety is recognized
    fn run_observer_safe(safe_ptr: SendPtr) {
        unsafe {
            Self::run_observer(safe_ptr.0);
        }
    }
}

#[cfg(target_os = "macos")]
pub async fn start_listener(tx: tokio::sync::mpsc::Sender<Option<SongInfo>>) {
    use tokio::process::Command;
    use tokio::sync::mpsc;

    // Internal channel to wake up Tokio when notified by C
    let (ping_tx, mut ping_rx) = mpsc::unbounded_channel::<()>();

    let thread_tx = SendWrapper(ping_tx.clone());
    let tx_ptr = Box::into_raw(Box::new(thread_tx));
    let safe_ptr = SendPtr(tx_ptr as *mut std::ffi::c_void);

    // Run the C loop in a dedicated thread to prevent blocking the async runtime
    std::thread::spawn(move || {
        MacManager::run_observer_safe(safe_ptr);
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
                return track_state & "|" & track_name & "|" & track_artist & "|" & track_duration & "|" & track_pos
            end if
        end tell
    "#;

    // This executes strictly when receiving a PING
    while (ping_rx.recv().await).is_some() {
        if let Ok(output) = Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .await 
        {
            let result = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !result.is_empty() {
                let parts: Vec<&str> = result.split('|').collect();
                if parts.len() == 5 {
                    let is_playing = parts[0] == "playing";
                    let duration_sec: f64 = parts[3].replace(",", ".").parse().unwrap_or(0.0);
                    let position_sec: f64 = parts[4].replace(",", ".").parse().unwrap_or(0.0);

                    let song = SongInfo::new(
                        parts[1], 
                        parts[2],
                        (duration_sec * 1000.0) as u32,
                        is_playing,
                        (position_sec * 1000.0) as u32,
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

                let timeline = session.GetTimelineProperties().ok()?;
                let length_ms = (timeline.EndTime().ok()?.Duration / 10_000) as u32;
                let position_ms = (timeline.Position().ok()?.Duration / 10_000) as u32;

                Some(SongInfo::new(
                    &title, 
                    &artist, 
                    length_ms, 
                    is_playing, 
                    position_ms
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