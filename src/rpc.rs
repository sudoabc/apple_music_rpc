use discord_rich_presence::{activity, DiscordIpc, DiscordIpcClient};
use crate::fetch::SongInfo;

pub struct DiscordClient {
    client: DiscordIpcClient
}

impl DiscordClient {
    pub fn new(client_id: &str) -> Self {
        let client = DiscordIpcClient::new(client_id);
        Self { client }
    }

    pub fn connect(&mut self) {
        match self.client.connect() {
            Ok(_) => println!("Connected to Discord IPC."),
            Err(e) => eprintln!("Failed to connect to Discord IPC: {}", e),
        }
    }

    pub fn clear_presence(&mut self) {
        let activity = activity::Activity::new()
            .name("Apple Music")
            .activity_type(activity::ActivityType::Listening)
            .details("Apple Music")
            .state("No Song Playing")
            .assets(
                activity::Assets::new()
                    .large_image("apple_music_logo")
                    .large_text("Apple Music")
            );
        match self.client.set_activity(activity) {
            Ok(_) => println!("Cleared Discord presence."),
            Err(e) => eprintln!("Failed to clear Discord presence: {}", e),
        }
    }

    pub fn update_presence(&mut self, song: &SongInfo) {
        let mut activity = activity::Activity::new()
            .name(&song.title)
            .activity_type(activity::ActivityType::Listening)
            .details(&song.title)
            .state(format!("by {}", song.artist))
            .details_url(song.song_url.clone().unwrap_or_else(|| "https://music.apple.com".to_string()))
            .assets(
                activity::Assets::new()
                    .large_image(song.cover_url.as_deref().unwrap_or("apple_music_logo"))
                    .large_text(format!("{} by {}", song.title, song.artist))
                    .small_image("apple_music_logo")
                    .small_text("Apple Music")
            )
            .buttons(vec![
                activity::Button::new(
                    "Play on Apple Music", 
                    song.song_url.clone().unwrap_or_else(|| "https://music.apple.com".to_string())
                )
            ]);

        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    
        if song.is_playing {
            let start_ts = now - (song.position_ms as u64 / 1000);
            let end_ts = start_ts + (song.length_ms as u64 / 1000);
            
            activity = activity.timestamps(
                activity::Timestamps::new()
                    .start(start_ts as i64)
                    .end(end_ts as i64)
            );
        } else {
            activity = activity.timestamps(
                activity::Timestamps::new()
                    .start(1)
                    .end(1)  
            );
        }
        let _ = self.client.set_activity(activity);
    }
}