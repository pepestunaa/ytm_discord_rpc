#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use discord_presence::{models::ActivityType, Client as DiscordClient};
use lru::LruCache;
use serde::Deserialize;
use std::num::NonZeroUsize;
use std::{env, time};
use std::{
    thread::{self},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use urlencoding::encode;
use winreg::enums::*;
use winreg::RegKey;

use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSessionManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus,
};

#[derive(Debug)]
struct MediaInfo {
    title: String,
    artist: String,
    start_timestamp: u64,
    end_timestamp: u64,
    is_playing: bool,
}
fn get_current_media() -> Option<MediaInfo> {
    let manager = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()
        .ok()?
        .get()
        .ok()?;
    let session = manager.GetCurrentSession().ok()?;

    let playback = session.GetPlaybackInfo().ok()?;
    let status = playback.PlaybackStatus().ok()?;
    let is_playing = status == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing;

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

    Some(MediaInfo {
        title,
        artist,
        start_timestamp: now_secs.saturating_sub(real_position_secs),
        end_timestamp: now_secs.saturating_sub(real_position_secs) + duration_secs,
        is_playing,
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

#[derive(Deserialize)]
struct ItunesResponse {
    results: Vec<ItunesTrack>,
}

#[derive(Deserialize)]
struct ItunesTrack {
    #[serde(rename = "artworkUrl100")]
    artwork_url_100: Option<String>,
}

fn get_album_art_url(title: &str, artist: &str) -> Option<String> {
    let query = format!("{} {}", title, artist);
    let url = format!(
        "https://itunes.apple.com/search?term={}&media=music&limit=1",
        encode(&query)
    );

    let body = ureq::get(&url)
        .timeout(time::Duration::from_secs(5))
        .call()
        .ok()?
        .into_string()
        .ok()?;

    let result: ItunesResponse = serde_json::from_str(&body).ok()?;

    result
        .results
        .first()?
        .artwork_url_100
        .as_ref()
        .map(|url| url.replace("100x100bb", "512x512bb"))
}

fn add_to_startup() {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let path = r#"Software\Microsoft\Windows\CurrentVersion\Run"#;

    if let Ok(key) = hkcu.open_subkey_with_flags(path, KEY_SET_VALUE) {
        if let Ok(exe_path) = env::current_exe() {
            let exe_path_str = exe_path.to_string_lossy().into_owned();
            let _ = key.set_value("ytm_discord_rpc", &exe_path_str);
        }
    }
}
fn main() {
    add_to_startup();
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
    let mut is_idle = false;
    let mut art_cache: LruCache<String, Option<String>> =
        LruCache::new(NonZeroUsize::new(20).unwrap());
    println!("Monitoring Windows SMTC...");
    loop {
        thread::sleep(Duration::from_millis(500));
        let Some(media) = get_current_media() else {
            // No media session at all — clear activity
            if !is_idle {
                println!("No media session found. Clearing activity.");
                let _ = drpc.clear_activity();
                is_idle = true;
            }
            continue;
        };

        // If music is paused or stopped, clear the activity
        if !media.is_playing {
            if !is_idle {
                println!("Music paused/stopped. Clearing activity.");
                let _ = drpc.clear_activity();
                is_idle = true;
            }
            continue;
        }

        // Music is playing — restore activity if needed
        if is_idle {
            println!("Music resumed. Restoring activity.");
            is_idle = false;
            needs_update = true;
        }

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

            // Fetch album art (with cache)
            let cache_key = format!("{}|{}", title, artist);
            let thumbnail_url = if let Some(cached) = art_cache.get(&cache_key) {
                cached
                    .clone()
                    .unwrap_or_else(|| "https://c.tenor.com/1gdoP8gQoqoAAAAC/tenor.gif".to_string())
            } else {
                // println!("Fetching album art for: {} - {}", title, artist);
                let art = get_album_art_url(&title, &artist);
                art_cache.put(cache_key.clone(), art.clone());
                // match &art {
                //     Some(url) => println!("Album art found: {}", url),
                //     None => println!("No album art found, using default logo"),
                // }
                art.unwrap_or_else(|| "https://c.tenor.com/1gdoP8gQoqoAAAAC/tenor.gif".to_string())
            };

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
                        a.large_image(&thumbnail_url)
                            .large_text("YouTube Music")
                            .small_image("https://c.tenor.com/I0IYOKVklREAAAAC/tenor.gif")
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
