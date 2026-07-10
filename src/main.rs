#![cfg_attr(target_os = "windows", windows_subsystem = "windows")] // Hides terminal on windows
mod fetch;
mod rpc;

use tray_icon::{menu::{Menu, MenuItem}, TrayIconBuilder, menu::MenuEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};

use std::time::Duration;
use tokio::time::sleep;

pub async fn run_rpc_loop() {
    let mut discord = rpc::DiscordClient::new("1525169580878594229");
    discord.connect();

    let mut last_song_title = String::new();
    let mut last_was_playing = true;

    loop {
        if let Some(mut song_info) = fetch::get_current_song().await {
            song_info.fetch_api_data().await;

            let song_changed = song_info.title != last_song_title;
            let status_changed = song_info.is_playing != last_was_playing;
            
            if song_changed || status_changed {
                discord.update_presence(&song_info);
                
                println!("Updated Discord presence for: {}", song_info.title);
                
                last_song_title = song_info.title.clone();
                last_was_playing = song_info.is_playing;
            }
        } else {
            if !last_song_title.is_empty() {
                discord.clear_presence();
                last_song_title = String::new();
                println!("No song playing, cleared presence.");
            }
        }

        sleep(Duration::from_secs(2)).await;
    }
}

#[tokio::main]
async fn main() {
    // 1. Setup Tray
    let tray_menu = Menu::new();
    let quit_i = MenuItem::new("Exit", true, None);
    let _ = tray_menu.append(&quit_i);

    let icon_bytes = include_bytes!("../assets/apple_music.png");
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

    // Start the icp loop in a separate thread
    std::thread::spawn(|| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            run_rpc_loop().await;
        });
    });

    event_loop.run(move |_event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        if let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == quit_i.id() {
                std::process::exit(0);
            }
        }
    });
}