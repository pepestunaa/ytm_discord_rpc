#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use discord_presence::{models::ActivityType, Client as DiscordClient};
use std::{
    thread::{self},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
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
    status: GlobalSystemMediaTransportControlsSessionPlaybackStatus,
}

fn get_current_media(
    manager: &GlobalSystemMediaTransportControlsSessionManager,
) -> Option<MediaInfo> {
    let session = manager.GetCurrentSession().ok()?;

    let playback = session.GetPlaybackInfo().ok()?;

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

    let status = playback.PlaybackStatus().ok()?;

    Some(MediaInfo {
        title,
        artist,
        start_timestamp: now_secs.saturating_sub(real_position_secs),
        end_timestamp: now_secs.saturating_sub(real_position_secs) + duration_secs,
        status,
    })
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

    let manager = match GlobalSystemMediaTransportControlsSessionManager::RequestAsync() {
        Ok(async_op) => match async_op.get() {
            Ok(mgr) => mgr,
            Err(e) => {
                eprintln!("Failed to get SMTC Manager: {:?}", e);
                return;
            }
        },
        Err(e) => {
            eprintln!("Failed to request SMTC Manager: {:?}", e);
            return;
        }
    };

    let mut last_track = String::new();
    let mut is_idle = false;
    let mut current_track = String::new();

    println!("Monitoring Windows SMTC...");

    loop {
        let Some(media) = get_current_media(&manager) else {
            thread::sleep(Duration::from_millis(500));
            continue;
        };
        if current_track != media.title {
            println!("Listening: {} - {}", media.title, media.artist);
            println!("Last track: {}", last_track);
            last_track = media.title.clone();
            current_track = media.title.clone();
            is_idle = false;
        }

        if media.status != GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing {
            if !is_idle {
                println!("No media playing. Setting status to Idle.");
                let res = drpc.set_activity(|act| {
                    act.activity_type(ActivityType::Playing)
                        .details("Idling")
                        .state("Chilling / No music playing")
                        .assets(|a| a.large_image("ytm_logo").large_text("Sleeptime"))
                });

                if res.is_ok() {
                    last_track.clear();
                    is_idle = true;
                }
            }
            continue;
        }
        
        let title = media.title.clone();
        let artist = media.artist.clone();
        let start = media.start_timestamp;
        let end = media.end_timestamp;

        let artist_url = format!(
            "https://music.youtube.com/search?q={}",
            artist.replace(' ', "+")
        );

        let listen_url = format!(
            "https://music.youtube.com/search?q={}+{}",
            title.replace(' ', "+"),
            artist.replace(' ', "+"),
        );

        let res = drpc.set_activity(|act| {
            act.activity_type(ActivityType::Listening)
                .details(title)
                .state(artist)
                .timestamps(|t| t.start(start).end(end))
                .assets(|a| {
                    a.large_image("ytm_logo")
                        .large_text("YouTube Music")
                        .small_image("1")
                        .small_text("Bcurretnt_track")
                })
                .append_buttons(|b| b.label("Listen Along").url(listen_url))
                .append_buttons(|b| b.label("View Artist").url(artist_url))
        });
        res.ok();
    }
}
