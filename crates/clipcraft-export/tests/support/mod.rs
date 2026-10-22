//! Shared generated fixtures: H.264/AAC clips made in Rust, so no capture or GPU is needed.
#![allow(dead_code)]
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use clipcraft_record::audio::AudioEngine;
use clipcraft_record::replay::EncodedSample;
use clipcraft_record::{
    AudioConfig, AudioControl, AudioError, AudioFormat, AudioInput, AudioSetup, AudioSource,
    ClipAudio, ClipFormat, VideoInfo, write_clip,
};
use filmcraft_h264enc::{
    Encoder, EncoderConfig, PacketFormat, Preset, Profile, RateControl, YuvFrame,
};
use filmcraft_media::reader::{FileReader, open_reader_within};
use filmcraft_media::{SharedReader, SharedSource};

pub struct Scratch(pub PathBuf);
impl Scratch {
    pub fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("clipcraft-compress-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    pub fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn source(path: &Path) -> SharedSource {
    let reader: SharedReader = Arc::new(FileReader::open(path).unwrap());
    open_reader_within(
        path.file_name().unwrap().to_str().unwrap(),
        reader,
        &filmcraft_codecs::reader_openers(),
        &filmcraft_codecs::openers(),
        64 * 1024 * 1024,
    )
    .unwrap()
}

struct TestSound {
    emitted: u64,
    click: bool,
}
impl AudioSource for TestSound {
    fn name(&self) -> String {
        "Generated audio".into()
    }
    fn format(&self) -> AudioFormat {
        AudioFormat {
            sample_rate: 48_000,
            channels: 2,
        }
    }
    fn read(&mut self, out: &mut Vec<f32>) -> Result<(), AudioError> {
        for i in self.emitted..self.emitted + 800 {
            let clicking = (96_000..96_480).contains(&i);
            let level = if self.click {
                if clicking { 0.9 } else { 0.0 }
            } else {
                0.2
            };
            let value =
                (std::f64::consts::TAU * 1_000.0 * i as f64 / 48_000.0).sin() as f32 * level;
            out.extend([value, value]);
        }
        self.emitted += 800;
        Ok(())
    }
}

pub fn fixture(path: &Path, format: ClipFormat, audio: bool) {
    fixture_sized(path, format, audio, (160, 90, 60), 4);
}

/// A clip of `seconds` at `(width, height, fps)`: moving noise on the right half, so it neither
/// compresses to nothing nor repeats.
pub fn fixture_sized(
    path: &Path,
    format: ClipFormat,
    audio: bool,
    (width, height, fps): (u32, u32, u32),
    seconds: u32,
) {
    let mut config = EncoderConfig::new(width, height, fps, 1);
    config.profile = Profile::Baseline;
    config.bframes = 0;
    config.threads = 2;
    config.preset = Preset::Speed;
    config.keyint = fps;
    config.format = PacketFormat::LengthPrefixed;
    config.rate = RateControl::Qp(6);
    let mut encoder = Encoder::new(config).unwrap();
    let video = VideoInfo {
        width,
        height,
        fps,
        avcc: encoder.avcc(),
    };
    let mut sound = AudioEngine::new(AudioSetup {
        config: AudioConfig {
            sample_rate: 48_000,
            channels: 2,
            kbps: 128,
            tracks: vec!["Click".into(), "Tone".into()],
        },
        inputs: vec![
            AudioInput::new(
                Box::new(TestSound {
                    emitted: 0,
                    click: true,
                }),
                AudioControl::default(),
                0,
            ),
            AudioInput::new(
                Box::new(TestSound {
                    emitted: 0,
                    click: false,
                }),
                AudioControl::default(),
                1,
            ),
        ],
    })
    .unwrap();
    let mut tracks: Vec<ClipAudio> = sound
        .tracks()
        .into_iter()
        .map(|info| ClipAudio {
            info,
            packets: Vec::new(),
        })
        .collect();
    let mut samples = Vec::new();
    let mut noise = 17_u32;
    let chroma = vec![128; width as usize * height as usize / 4];
    for i in 0..fps * seconds {
        let mut luma = vec![0; width as usize * height as usize];
        for (pixel, y) in luma.iter_mut().enumerate() {
            noise = noise.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *y = if pixel % (width as usize) < width as usize / 2 {
                40 + (i / fps) as u8 * 45
            } else {
                (noise >> 24) as u8
            };
        }
        for p in encoder
            .encode(
                &YuvFrame {
                    y: &luma,
                    u: &chroma,
                    v: &chroma,
                    y_stride: width as usize,
                    uv_stride: width as usize / 2,
                },
                i64::from(i),
            )
            .unwrap()
        {
            samples.push(EncodedSample {
                pts: p.pts,
                data: p.data.into(),
                keyframe: p.keyframe,
            });
        }
        if audio {
            for (track, packet) in sound.tick(Duration::from_nanos(
                u64::from(i + 1) * 1_000_000_000 / u64::from(fps),
            )) {
                tracks[track].packets.push(packet);
            }
        }
    }
    for p in encoder.flush() {
        samples.push(EncodedSample {
            pts: p.pts,
            data: p.data.into(),
            keyframe: p.keyframe,
        });
    }
    write_clip(
        &samples,
        &video,
        if audio { &tracks } else { &[] },
        format,
        path,
    )
    .unwrap();
}
