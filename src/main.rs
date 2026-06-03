// #![windows_subsystem = "windows"]
use discord_presence::{models::ActivityType, Client as DiscordClient};
use std::{
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSessionManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus,
};

struct MediaInfo {
    title: String,
    artist: String,
    start_timestamp: u64,
    end_timestamp: u64,
}

fn get_current_media() -> Option<MediaInfo> {
    let manager = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()
        .ok()?
        .get()
        .ok()?;

    let session = manager.GetCurrentSession().ok()?;

    // Hanya tampilkan saat lagu benar-benar playing
    let playback = session.GetPlaybackInfo().ok()?;
    if playback.PlaybackStatus().ok()?
        != GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing
    {
        return None;
    }

    let props = session.TryGetMediaPropertiesAsync().ok()?.get().ok()?;
    let title = props.Title().ok()?.to_string();
    let artist = props.Artist().ok()?.to_string();

    if title.is_empty() {
        return None;
    }

    // Ambil posisi dan durasi untuk progress bar Discord
    let timeline = session.GetTimelineProperties().ok()?;
    let position_secs = (timeline.Position().ok()?.Duration / 10_000_000) as u64;
    let duration_secs = (timeline.EndTime().ok()?.Duration / 10_000_000) as u64;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    Some(MediaInfo {
        title,
        artist,
        start_timestamp: now.saturating_sub(position_secs),
        end_timestamp: now.saturating_sub(position_secs) + duration_secs,
    })
}

fn main() {
    // Pastikan nama aplikasi di Discord Developer Portal diset ke "YouTube Music"
    let mut drpc = DiscordClient::new(1511746527125700842);

    drpc.on_ready(|_ctx| {
        println!("Berhasil terhubung ke Discord Rich Presence!");
    })
    .persist();

    drpc.start();

    println!("Menunggu koneksi ke Discord...");
    while !DiscordClient::is_ready() {
        thread::sleep(Duration::from_millis(500));
    }

    let mut last_track = String::new();
    let mut no_media_streak = 0u32;
    let mut was_ready = false;

    println!("Mulai memonitor Windows SMTC...");

    loop {
        let is_ready = DiscordClient::is_ready();
        // Jika Discord ditutup/disconnect, bersihkan track agar mengulang saat buka lagi
        if is_ready && !was_ready {
            println!("Discord terdeteksi siap! Mereset status untuk sinkronisasi ulang...");
            last_track.clear();
            was_ready = true;
        }

        if !is_ready {
            if was_ready {
                println!("Discord terdeteksi tidak siap! Menunggu koneksi...");
                was_ready = false;
            }
            last_track.clear();
            thread::sleep(Duration::from_millis(1000));
            continue;
        }

        if let Some(media) = get_current_media() {
            no_media_streak = 0;

            if media.title != last_track {
                println!("Mendengar: {} - {}", media.title, media.artist);

                let title = media.title.clone();
                let artist = media.artist.clone();
                let start = media.start_timestamp;
                let end = media.end_timestamp;

                // Membuat URL search artis otomatis
                let artist_url = format!(
                    "https://music.youtube.com/search?q={}",
                    artist.replace(' ', "+")
                );

                // Kirim Rich Presence ke Discord
                let res = drpc.set_activity(|act| {
                    act.activity_type(ActivityType::Listening)
                        .details(title)
                        .state(artist)
                        .timestamps(|t| t.start(start).end(end))
                        .assets(|a| a.large_image("ytm_logo").large_text("YouTube Music"))
                        .append_buttons(|b| b.label("View Artist").url(artist_url))
                });

                match res {
                    Ok(_) => last_track = media.title,
                    Err(e) => {
                        println!(
                            "Gagal update ke Discord, kemungkinan Discord ditutup: {:?}",
                            e
                        );
                        // Fitur Auto-Close: Matikan program background ini jika Discord ditutup
                        last_track.clear();
                    }
                }
            }
        } else {
            no_media_streak += 1;
            if no_media_streak >= 3 && !last_track.is_empty() {
                println!("Media berhenti. Menghapus status Discord.");
                let _ = drpc.clear_activity();
                last_track.clear();
                no_media_streak = 0;
            }
        }

        thread::sleep(Duration::from_secs(5));
    }
}
