#![cfg_attr(target_os = "windows", windows_subsystem = "windows")] // Hides terminal on windows
mod fetch;
mod rpc;

use tray_icon::{
    TrayIconBuilder,
    menu::MenuEvent,
    menu::{Menu, MenuItem}
};
use tao::event_loop::{ControlFlow, EventLoopBuilder};

pub async fn run_rpc() {
    let mut discord: rpc::DiscordClient = rpc::DiscordClient::new("1525169580878594229");
    let mut last_song_title: String = String::new();
    let mut last_was_playing: bool = true;
    let mut last_position: u32 = 0;

    // Creating the communication pipe
    let (tx, mut rx) = tokio::sync::mpsc::channel(32);

    tokio::spawn(async move {
        fetch::start_listener(tx).await;
    });

    println!("[RPC] Started listening for Apple Music events...");

    while let Some(event) = rx.recv().await {
        match event {
            Some(mut song_info) => {
                if !discord.is_connected() {
                    discord.connect();
                }

                let song_changed = song_info.title != last_song_title;
                let status_changed = song_info.is_playing != last_was_playing;
                let seeked_or_repeated = song_info.is_playing 
                    && last_was_playing 
                    && song_info.position_ms < (last_position.saturating_sub(3000));

                if song_changed || status_changed || seeked_or_repeated {
                    song_info.fetch_api_data().await;
                    discord.update_presence(&song_info);
                    println!("[RPC] Updated Discord presence for: {}", song_info.title);
                    last_song_title = song_info.title.clone();
                    last_was_playing = song_info.is_playing;
                }
                last_position = song_info.position_ms;
            },
            None => {
                if discord.is_connected() {
                    discord.disconnect();
                    last_song_title = String::new();
                    println!("[RPC] No song playing, cleared presence.");
                }
            }
        }
    }
}

fn main() {
    // Setup Tray
    let tray_menu = Menu::new();
    let quit_i = MenuItem::new("Exit", true, None);
    let _ = tray_menu.append(&quit_i);

    let icon_bytes = include_bytes!("../assets/apple_music.ico");
    let img = image::load_from_memory(icon_bytes).expect("Failed to open icon file");
    let width = img.width();
    let height = img.height();
    let rgba_data = img.into_rgba8().into_raw();

    let icon = tray_icon::Icon::from_rgba(rgba_data, width, height)
        .expect("Failed to create icon from bytes");
    
    let _tray = TrayIconBuilder::new()
        .with_menu(Box::new(tray_menu))
        .with_tooltip("Apple Music")
        .with_icon(icon)
        .build()
        .unwrap();

    let event_loop = EventLoopBuilder::new().build();

    // Start the RPC loop
    std::thread::spawn(|| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            run_rpc().await;
        });
    });

    // UI Event Loop
    event_loop.run(move |_event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        if let Ok(event) = MenuEvent::receiver().try_recv() 
            && event.id == quit_i.id() 
        {
            std::process::exit(0);
        }
        
    });
}