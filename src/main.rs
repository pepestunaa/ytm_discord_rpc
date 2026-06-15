#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use discord_presence::{models::ActivityType, Client as DiscordClient};
use std::{
    thread::{self},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use urlencoding::encode;
use windows::{
    Media::Control::GlobalSystemMediaTransportControlsSessionManager,
    // Win32::Foundation::D2DERR_TEXT_RENDERER_NOT_RELEASED,
};

#[derive(Debug)]
struct MediaInfo {
    title: String,
    artist: String,
    start_timestamp: u64,
    end_timestamp: u64,
    // status: GlobalSystemMediaTransportControlsSessionPlaybackStatus,
}
fn get_current_media() -> Option<MediaInfo> {
    let manager = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()
        .ok()?
        .get()
        .ok()?;
    let session = manager.GetCurrentSession().ok()?;

    // let playback = session.GetPlaybackInfo().ok()?;

    let props = session.TryGetMediaPropertiesAsync().ok()?.get().ok()?;
    let title = props.Title().ok()?.to_string();
    let artist = props.Artist().ok()?.to_string();

    if title.is_empty() {
        return None;
    }

    let timeline = session.GetTimelineProperties().ok()?;

    let position_ticks = timeline.Position().ok()?.Duration;
    let duration_ticks = timeline.EndTime().ok()?.Duration;

    let last_updated_filetime = timeline.LastUpdatedTime().ok()?.UniversalTime;

    let last_updated_secs = (last_updated_filetime / 10_000_000) as u64 - 11_644_473_600;

    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let time_drift = now_secs.saturating_sub(last_updated_secs);
    let position_secs = (position_ticks / 10_000_000) as u64;
    let duration_secs = (duration_ticks / 10_000_000) as u64;
    let real_position_secs = position_secs + time_drift;

    // let status = playback.PlaybackStatus().ok()?;

    Some(MediaInfo {
        title,
        artist,
        start_timestamp: now_secs.saturating_sub(real_position_secs),
        end_timestamp: now_secs.saturating_sub(real_position_secs) + duration_secs,
        // status,
    })
}
fn format_media_str(text: &str) -> String {
    let mut s = text.trim().to_string();
    if s.is_empty() {
        return "Unknown text".to_string();
    }
    if s.chars().count() < 2 {
        s.push(' ');
    }
    if s.chars().count() > 128 {
        let mut truncated: String = s.chars().take(125).collect();
        truncated.push_str("...");
        return truncated;
    }
    s
}
fn main() {
    let mut drpc = DiscordClient::new(1511746527125700842);
    drpc.on_ready(|_ctx| {
        println!("Discord Rich Presence Connected!");
    })
    .persist();
    drpc.start();
    println!("Connecting to Discord...");
    while !DiscordClient::is_ready() {
        thread::sleep(Duration::from_millis(500));
    }
    let mut current_track = String::new();
    let mut current_start = None;
    let mut needs_update = true;
    println!("Monitoring Windows SMTC...");
    thread::sleep(Duration::from_millis(500));
    loop {
        thread::sleep(Duration::from_millis(500));
        let Some(media) = get_current_media() else {
            thread::sleep(Duration::from_millis(500));
            continue;
        };
        if current_track != media.title || current_start != Some(media.start_timestamp) {
            println!("Listening: {} - {}", media.title, media.artist);
            current_track = media.title.clone();
            current_start = Some(media.start_timestamp);
            needs_update = true;
        }
        if needs_update {
            let title = format_media_str(&media.title);
            let artist = format_media_str(&media.artist);
            let start = media.start_timestamp;
            let end = media.end_timestamp;
            let search_artist = format!("{}", artist);
            let search_listen = format!("{} {}", title, artist);

            let artist_url = format!(
                "https://music.youtube.com/search?q={}",
                encode(&search_artist)
            );

            let listen_url = format!(
                "https://music.youtube.com/search?q={}",
                encode(&search_listen)
            );
            let res = drpc.set_activity(|act| {
                act.activity_type(ActivityType::Listening)
                    .details(&title)
                    .state(&artist)
                    .timestamps(|t| t.start(start).end(end))
                    .assets(|a| {
                        a.large_image("ytm_logo")
                            // .large_text("YouTube Music")
                            .small_image("1")
                            .small_text("Bub")
                    })
                    .append_buttons(|b| b.label("Listen Along").url(listen_url))
                    .append_buttons(|b| b.label("View Artist").url(artist_url))
            });
            match res {
                Ok(_) => {
                    println!("a");
                    needs_update = false;
                }
                Err(e) => {
                    eprintln!("Failed to set activity: {}", e);
                }
            }
            thread::sleep(Duration::from_secs(10));
        }
        println!("b");
        thread::sleep(Duration::from_secs(10));
    }
}
