use discord_rich_presence::{DiscordIpc, DiscordIpcClient, activity};
use crate::fetch::SongInfo;

pub struct DiscordClient {
    client: DiscordIpcClient,
    connected: bool
}

impl DiscordClient {
    pub fn new(client_id: &str) -> Self {
        let client = DiscordIpcClient::new(client_id);
        Self { 
            client,
            connected: false,
        }
    }

    pub fn is_connected(&self) -> bool {
        self.connected
    }

    pub fn connect(&mut self) {
        if self.connected { 
            return; 
        }

        match self.client.connect() {
            Ok(_) => {
                println!("Connected to Discord IPC.");
                self.connected = true;
            }
            Err(e) => eprintln!("Failed to connect to Discord IPC: {}", e),
        }
    }

    pub fn disconnect(&mut self) {
        if !self.connected { 
            return; 
        }
        match self.client.close() {
            Ok(_) => {
                println!("Disconnected from Discord IPC.");
                self.connected = false;
            }
            Err(e) => eprintln!("Failed to disconnect from Discord IPC: {}", e),
        }
    }

    pub fn update_presence(&mut self, song: &SongInfo) {
        if !self.connected { 
            return; 
        }

        let mut activity = activity::Activity::new()
            .name(&song.title)
            .activity_type(activity::ActivityType::Listening)
            .details(&song.title)
            .state(format!("by {}", song.artist))
            .details_url(
                song.song_url
                    .clone()
                    .unwrap_or_else(|| "https://music.apple.com".to_string()),
                )
            .assets(
                activity::Assets::new()
                    .large_image(song.cover_url.as_deref().unwrap_or("apple_music_logo"))
                    .large_text(format!("{} by {}", song.title, song.artist))
                    .small_image("apple_music_logo")
                    .small_text("Apple Music"),
            )
            .buttons(vec![
                activity::Button::new(
                    "Play on Apple Music", 
                    song.song_url.clone().unwrap_or_else(|| "https://music.apple.com".to_string())
                ),
                activity::Button::new(
                    "Github Repository",
                    "https://github.com/sudoabc/apple_music_rpc",
                ),
            ]);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
    
        if song.is_playing {
            let start_ts = now - (song.position_ms as u64 / 1000);
            let end_ts = start_ts + (song.length_ms as u64 / 1000);
            
            activity = activity.timestamps(
                activity::Timestamps::new()
                    .start(start_ts as i64)
                    .end(end_ts as i64),
            );
        }
        let _ = self.client.set_activity(activity);
    }
}