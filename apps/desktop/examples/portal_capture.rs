//! P0 spike, step 2: capture a real monitor through the desktop portal and
//! publish it to a local LiveKit server at 1920x1080.
//!
//! Run a server first: `livekit-server --dev`
//! Then: `cargo run --example portal_capture`
//!
//! The first run opens the system monitor picker. Choose a screen; the choice
//! is remembered by the portal for later runs.

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use ashpd::desktop::screencast::{
    CursorMode, OpenPipeWireRemoteOptions, Screencast, SelectSourcesOptions, SourceType,
    StartCastOptions,
};
use futures::StreamExt;
use gst::prelude::*;
use gstreamer as gst;
use gstreamer_app::AppSink;
use livekit::options::{TrackPublishOptions, VideoCodec, VideoEncoding};
use livekit::prelude::*;
use livekit::webrtc::video_frame::{BoxVideoFrame, I420Buffer, VideoFrame, VideoRotation};
use livekit::webrtc::video_stream::native::NativeVideoStream;
use livekit_capture::error::SourceError;
use livekit_capture::pixel::{PixelVideoPump, PixelVideoSource};
use livekit_capture::primitive::VideoResolution;
use livekit_capture::pump::PumpStop;
use livekit_token::{AccessToken, VideoGrants};

const URL: &str = "ws://127.0.0.1:7880";
const API_KEY: &str = "devkey";
const API_SECRET: &str = "secret";
const ROOM: &str = "spike";
const MEASURE: Duration = Duration::from_secs(8);

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

/// Pulls I420 frames from a `pipewiresrc` pipeline fed by the portal.
struct PortalScreen {
    // The portal session and its fd must outlive the stream they authorised:
    // the node id is only valid inside that PipeWire remote.
    _session: ashpd::desktop::Session<Screencast>,
    _remote: std::os::fd::OwnedFd,
    pipeline: gst::Pipeline,
    sink: AppSink,
    bus: gst::Bus,
    resolution: VideoResolution,
    captured: u64,
    reported_error: bool,
}

impl PortalScreen {
    async fn open() -> Result<Self> {
        gst::init().context("initialising GStreamer")?;

        let screencast = Screencast::new().await.context("connecting to the portal")?;
        let session = screencast
            .create_session(Default::default())
            .await
            .context("creating a screencast session")?;

        screencast
            .select_sources(
                &session,
                SelectSourcesOptions::default()
                    .set_sources(SourceType::MONITOR)
                    .set_multiple(false)
                    .set_cursor_mode(CursorMode::EMBEDDED),
            )
            .await
            .context("selecting a monitor")?;

        let streams = screencast
            .start(&session, None, StartCastOptions::default())
            .await
            .context("starting the screencast")?
            .response()
            .context("reading the portal response")?;

        let stream = streams.streams().first().context("the portal returned no stream")?;
        let node_id = stream.pipe_wire_node_id();
        let (width, height) = stream.size().unwrap_or((1920, 1080));
        println!("portal granted node {node_id} at {width}x{height}");

        // Keep the fd open for the lifetime of the pipeline: pipewiresrc
        // connects to the session-scoped remote, not the default daemon.
        let remote = screencast
            .open_pipe_wire_remote(&session, OpenPipeWireRemoteOptions::default())
            .await
            .context("opening the PipeWire remote")?;

        let src = gst::ElementFactory::make("pipewiresrc")
            .property("fd", std::os::fd::AsRawFd::as_raw_fd(&remote))
            .property("path", node_id.to_string())
            .build()
            .context("creating pipewiresrc")?;
        let convert = gst::ElementFactory::make("videoconvert")
            .build()
            .context("creating videoconvert")?;
        let caps = gst::Caps::builder("video/x-raw")
            .field("format", "I420")
            .build();
        let capsfilter = gst::ElementFactory::make("capsfilter")
            .property("caps", &caps)
            .build()
            .context("creating capsfilter")?;
        let sink = gstreamer_app::AppSink::builder()
            .sync(false)
            .max_buffers(2)
            .drop(true)
            .build();
        let sink_element = sink.clone().upcast::<gst::Element>();

        let pipeline = gst::Pipeline::new();
        pipeline
            .add_many([&src, &convert, &capsfilter, &sink_element])
            .context("assembling the pipeline")?;
        gst::Element::link_many([&src, &convert, &capsfilter, &sink_element])
            .context("linking the capture pipeline")?;

        pipeline
            .set_state(gst::State::Playing)
            .context("starting the capture pipeline")?;

        let bus = pipeline.bus().context("pipeline has no bus")?;

        Ok(Self {
            _session: session,
            _remote: remote,
            pipeline,
            sink,
            bus,
            resolution: VideoResolution { width: width as u32, height: height as u32 },
            captured: 0,
            reported_error: false,
        })
    }
}

impl PixelVideoSource for PortalScreen {
    fn resolution(&self) -> VideoResolution {
        self.resolution
    }

    fn next_frame(&mut self, stop: &PumpStop) -> Result<Option<BoxVideoFrame>, SourceError> {
        while !stop.is_stopped() {
            let Some(sample) = self.sink.try_pull_sample(gst::ClockTime::from_mseconds(100)) else {
                if !self.reported_error {
                    while let Some(msg) = self.bus.pop() {
                        use gst::MessageView;
                        match msg.view() {
                            MessageView::Error(err) => {
                                eprintln!(
                                    "pipeline error from {:?}: {} ({:?})",
                                    err.src().map(|s| s.name()),
                                    err.error(),
                                    err.debug()
                                );
                                self.reported_error = true;
                            }
                            MessageView::StateChanged(state) => {
                                if let Some(el) = state.src() {
                                    if el.name() == "pipeline0" {
                                        eprintln!(
                                            "pipeline state: {:?} -> {:?}",
                                            state.old(),
                                            state.current()
                                        );
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                continue;
            };
            let Some(buffer) = sample.buffer() else {
                continue;
            };
            let caps = sample
                .caps()
                .ok_or_else(|| SourceError::new(std::io::Error::other("sample has no caps")))?;
            let mapped = buffer
                .map_readable()
                .map_err(|_| SourceError::new(std::io::Error::other("buffer not readable")))?;

            let frame_info = gstreamer_video::VideoInfo::from_caps(caps)
                .map_err(|_| SourceError::new(std::io::Error::other("unparsable caps")))?;

            let (width, height) = (frame_info.width(), frame_info.height());
            let mut i420 = I420Buffer::new(width, height);
            let (stride_y, stride_u, stride_v) = i420.strides();
            let (dst_y, dst_u, dst_v) = i420.data_mut();

            // Offsets and strides come from the negotiated caps: PipeWire can
            // hand back padded rows, so a tight copy would shear the image.
            let copy_plane = |plane: usize, dst: &mut [u8], dst_stride: u32| {
                let src_stride = frame_info.stride()[plane] as usize;
                let rows = frame_info.height() as usize / if plane == 0 { 1 } else { 2 };
                for row in 0..rows {
                    let src_start = frame_info.offset()[plane] + row * src_stride;
                    let dst_start = row * dst_stride as usize;
                    let len = dst_stride as usize;
                    if src_start + len > mapped.size() || dst_start + len > dst.len() {
                        return;
                    }
                    dst[dst_start..dst_start + len]
                        .copy_from_slice(&mapped[src_start..src_start + len]);
                }
            };

            copy_plane(0, dst_y, stride_y);
            copy_plane(1, dst_u, stride_u);
            copy_plane(2, dst_v, stride_v);

            self.captured += 1;
            if self.captured % 60 == 0 {
                println!("captured {} frames from the portal", self.captured);
            }

            return Ok(Some(VideoFrame {
                rotation: VideoRotation::VideoRotation0,
                timestamp_us: 0,
                frame_metadata: None,
                buffer: Box::new(i420),
            }));
        }
        Ok(None)
    }
}

impl Drop for PortalScreen {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let source = PortalScreen::open().await?;

    let (publisher, _publisher_events) =
        Room::connect(URL, &token("publisher")?, RoomOptions::default()).await?;
    println!("publisher connected to {ROOM}");

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
                video_encoding: Some(VideoEncoding { max_bitrate: 6_000_000, max_framerate: 60.0 }),
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
