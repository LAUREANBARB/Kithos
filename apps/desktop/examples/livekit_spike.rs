//! P0 spike: publish a 1920x1080@60 synthetic track to a local LiveKit server
//! and measure what a second participant actually receives.
//!
//! Run a server first: `livekit-server --dev`
//! Then: `cargo run --example livekit_spike`

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use futures::StreamExt;
use livekit::options::{TrackPublishOptions, VideoCodec};
use livekit::prelude::*;
use livekit::webrtc::video_stream::native::NativeVideoStream;
use livekit_capture::pixel::PixelVideoPump;
use livekit_capture::primitive::VideoResolution;
use livekit_capture::sources::pattern::{Pattern, PatternVideoSource, PatternVideoSourceConfig};
use livekit_token::{AccessToken, VideoGrants};

const URL: &str = "ws://127.0.0.1:7880";
const API_KEY: &str = "devkey";
const API_SECRET: &str = "secret";
const ROOM: &str = "spike";
const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
const MEASURE: Duration = Duration::from_secs(8);

fn source_fps() -> u32 {
    std::env::var("KITHOS_SPIKE_FPS").ok().and_then(|v| v.parse().ok()).unwrap_or(60)
}

fn token(identity: &str) -> Result<String> {
    AccessToken::with_api_key(API_KEY, API_SECRET)
        .with_identity(identity)
        .with_grants(VideoGrants {
            room_join: true,
            room: ROOM.into(),
            can_publish: Some(true),
            can_subscribe: Some(true),
            ..Default::default()
        })
        .to_jwt()
        .context("signing the access token")
}

#[tokio::main]
async fn main() -> Result<()> {
    let (publisher, _publisher_events) =
        Room::connect(URL, &token("publisher")?, RoomOptions::default()).await?;
    println!("publisher connected to {ROOM}");

    let source = PatternVideoSource::new(PatternVideoSourceConfig {
        resolution: VideoResolution { width: WIDTH, height: HEIGHT },
        framerate_fps: source_fps(),
        pattern: Pattern::Gradient,
    })
    .await
    .context("building the pattern source")?;

    let pump = PixelVideoPump::new(source);
    let track = LocalVideoTrack::create_video_track("screen", pump.rtc_source());
    let publication = publisher
        .local_participant()
        .publish_track(
            LocalTrack::Video(track),
            TrackPublishOptions {
                source: TrackSource::Screenshare,
                video_codec: VideoCodec::H264,
                simulcast: std::env::var("KITHOS_SPIKE_SIMULCAST").is_err(),
                video_encoding: Some(livekit::options::VideoEncoding {
                    max_bitrate: 6_000_000,
                    max_framerate: 60.0,
                }),
                video_encoder: match std::env::var("KITHOS_SPIKE_ENCODER").as_deref() {
                    Ok("vaapi") => livekit::options::VideoEncoderBackend::Vaapi,
                    Ok("hardware") => livekit::options::VideoEncoderBackend::Hardware,
                    Ok("software") => livekit::options::VideoEncoderBackend::Software,
                    _ => livekit::options::VideoEncoderBackend::Auto,
                },
                ..Default::default()
            },
        )
        .await
        .context("publishing the screen track")?;
    println!("published track {}", publication.sid());

    let running_pump = pump.spawn().context("spawning the frame pump")?;

    let (viewer, mut events) =
        Room::connect(URL, &token("viewer")?, RoomOptions::default()).await?;
    println!("viewer connected to {ROOM}");

    let deadline = Instant::now() + MEASURE;
    let mut measured = false;

    while Instant::now() < deadline && !measured {
        let Ok(Some(event)) = tokio::time::timeout(Duration::from_secs(5), events.recv()).await else {
            continue;
        };

        let RoomEvent::TrackSubscribed { track, publication, .. } = event else {
            continue;
        };
        let RemoteTrack::Video(video) = track else {
            continue;
        };

        println!("viewer subscribed to {}", publication.sid());

        let mut stream = NativeVideoStream::new(video.rtc_track());
        let mut frames: u64 = 0;
        let mut size = (0, 0);
        let start = Instant::now();

        while start.elapsed() < MEASURE {
            let Ok(Some(frame)) = tokio::time::timeout(Duration::from_secs(2), stream.next()).await
            else {
                break;
            };
            frames += 1;
            size = (frame.buffer.as_ref().width(), frame.buffer.as_ref().height());
        }

        let secs = start.elapsed().as_secs_f64();
        println!(
            "received {frames} frames in {secs:.1}s = {:.1} fps at {}x{}",
            frames as f64 / secs,
            size.0,
            size.1
        );
        measured = true;
    }

    running_pump.stop();
    let _ = viewer.close().await;
    let _ = publisher.close().await;

    anyhow::ensure!(measured, "viewer never received the screen track");
    Ok(())
}
