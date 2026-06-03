#![windows_subsystem = "windows"]
use discord_presence::{models::ActivityType, Client as DiscordClient};
use std::{
    collections::HashMap,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSessionManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus,
};
use windows::Storage::Streams::DataReader;

struct MediaInfo {
    title: String,
    artist: String,
    start_timestamp: u64,
    end_timestamp: u64,
    /// Raw image bytes dari thumbnail SMTC
    thumbnail: Option<Vec<u8>>,
}

/// Baca bytes thumbnail dari SMTC stream
fn read_thumbnail(
    props: &windows::Media::Control::GlobalSystemMediaTransportControlsSessionMediaProperties,
) -> Option<Vec<u8>> {
    let thumbnail_ref = props.Thumbnail().ok()?;
    let stream = thumbnail_ref.OpenReadAsync().ok()?.get().ok()?;
    let size = stream.Size().ok()?;

    if size == 0 {
        return None;
    }

    let reader = DataReader::CreateDataReader(&stream).ok()?;
    let loaded = reader.LoadAsync(size as u32).ok()?.get().ok()?;

    if loaded == 0 {
        return None;
    }

    let mut bytes = vec![0u8; loaded as usize];
    reader.ReadBytes(&mut bytes).ok()?;

    Some(bytes)
}

/// Upload gambar ke 0x0.st (gratis, tanpa API key) dan kembalikan URL-nya
fn upload_image(bytes: &[u8]) -> Option<String> {
    let boundary = "----DiscordRPCBoundary";

    // Buat multipart/form-data body secara manual
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\n\
             Content-Disposition: form-data; name=\"file\"; filename=\"thumb.jpg\"\r\n\
             Content-Type: image/jpeg\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let resp = ureq::post("https://0x0.st")
        .set(
            "Content-Type",
            &format!("multipart/form-data; boundary={boundary}"),
        )
        .set("User-Agent", "ytm-discord-rpc/0.1")
        .send_bytes(&body)
        .ok()?;

    if resp.status() == 200 {
        let url = resp.into_string().ok()?.trim().to_string();
        if url.starts_with("https://") {
            return Some(url);
        }
    }

    None
}

fn get_current_media() -> Option<MediaInfo> {
    let manager = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()
        .ok()?
        .get()
        .ok()?;

    let session = manager.GetCurrentSession().ok()?;

    // Hanya tampilkan saat lagu benar-benar playing (bukan paused atau stopped)
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

    let thumbnail = read_thumbnail(&props);

    // Ambil posisi dan durasi untuk progress bar Discord
    // TimeSpan.Duration satuannya 100-nanosecond intervals
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
        thumbnail,
    })
}

fn main() {
    // Pastikan nama aplikasi di Discord Developer Portal diset ke "YouTube Music"
    // agar Discord menampilkan "Listening to YouTube Music"
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
    // Cache URL thumbnail yang sudah diupload: "title - artist" → URL
    let mut thumbnail_cache: HashMap<String, String> = HashMap::new();

    println!("Mulai memonitor Windows SMTC...");

    loop {
        // Jika Discord disconnect, reset agar activity dikirim ulang saat reconnect
        if !DiscordClient::is_ready() {
            println!("Discord disconnect, menunggu reconnect...");
            last_track.clear();
            thread::sleep(Duration::from_millis(500));
            continue;
        }

        if let Some(media) = get_current_media() {
            no_media_streak = 0;

            if media.title != last_track {
                println!("Mendengar: {} - {}", media.title, media.artist);

                // Dapatkan URL gambar: dari cache atau upload baru
                let cache_key = format!("{} - {}", media.title, media.artist);
                let image_url = if let Some(bytes) = &media.thumbnail {
                    match upload_image(bytes) {
                        Some(url) => {
                            println!("  Thumbnail: {}", url);
                            thumbnail_cache.insert(cache_key, url.clone());
                            url
                        }
                        None => {
                            "ytm_logo".to_string()
                        }
                    }
                } else {
                    "ytm_logo".to_string()
                };

                // Salin nilai ke variabel lokal agar bisa di-move ke dalam closure
                let title = media.title.clone();
                let artist = media.artist.clone();
                let start = media.start_timestamp;
                let end = media.end_timestamp;
                let artist_url = format!(
                    "https://music.youtube.com/search?q={}",
                    artist.replace(' ', "+")
                );

                let res = drpc.set_activity(|act| {
                    act.activity_type(ActivityType::Listening)
                        .details(title)
                        .state(artist)
                        .timestamps(|t| t.start(start).end(end))
                        .assets(|a| a.large_image(image_url).large_text("YouTube Music"))
                        .append_buttons(|b| b.label("View Artist").url(artist_url))
                });

                match res {
                    Ok(_) => last_track = media.title,
                    Err(e) => println!("Gagal update ke Discord: {:?}", e),
                }
            }
        } else {
            no_media_streak += 1;
            // Tunggu 3 poll berturut-turut (~15 detik) sebelum clear, agar tidak
            // langsung hilang saat ganti lagu atau SMTC sesaat tidak responsif
            if no_media_streak >= 3 && !last_track.is_empty() {
                println!("Media berhenti. Menghapus status Discord.");
                let _ = drpc.clear_activity();
                last_track.clear();
                no_media_streak = 0;
            }
        }

        // Poll tiap 5 detik (dalam batas rate limit Discord: 5 update per 20 detik)
        thread::sleep(Duration::from_secs(5));
    }
}
