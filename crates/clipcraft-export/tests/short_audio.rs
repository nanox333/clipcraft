//! A clip whose sound ends before its picture can still be cut after the sound has ended.
use std::io::Cursor;
use std::sync::atomic::AtomicBool;

use clipcraft_export::{TrimRequest, trim_to_mp4};
use clipcraft_time::Tick;
use filmcraft_isobmff::{
    AvcConfig, Brand, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions,
};

fn clip(video_frames: u32, audio_units: u32) -> Vec<u8> {
    let mut writer =
        Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).expect("writer");
    let entry = SampleEntry::avc(
        AvcConfig::new(
            vec![vec![0x67, 0x64, 0, 0x28, 1, 2]],
            vec![vec![0x68, 3]],
            4,
        ),
        1920,
        1080,
    );
    let video = writer.add_track(TrackConfig::new(entry, 30)).expect("video");
    let audio = writer
        .add_track(TrackConfig::new(
            SampleEntry::aac(vec![0x11, 0x90], 2, 48_000),
            48_000,
        ))
        .expect("audio");
    for index in 0..video_frames {
        let sample = WriteSample {
            data: &[0, 0, 0, 2, 0x65, 0],
            duration: 1,
            composition_offset: 0,
            is_sync: index % 30 == 0,
        };
        writer.write_sample(video, sample).expect("picture");
    }
    for _ in 0..audio_units {
        let sample = WriteSample {
            data: &[1, 2, 3],
            duration: 1_024,
            composition_offset: 0,
            is_sync: true,
        };
        writer.write_sample(audio, sample).expect("sound");
    }
    writer.finish().expect("finish").into_inner()
}

#[test]
fn a_cut_after_the_sound_has_ended_keeps_the_picture() {
    let dir = std::env::temp_dir().join(format!("clipcraft-short-audio-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    let input = dir.join("in.mp4");
    // Three seconds of picture, one second of sound.
    std::fs::write(&input, clip(90, 47)).expect("write");
    for (name, start, end) in [("late", 2.0, 2.9), ("mid", 1.5, 2.5), ("early", 0.0, 1.0)] {
        let request = TrimRequest {
            input: input.clone(),
            output: dir.join(format!("{name}.mp4")),
            start: Tick::from_seconds_f64(start),
            end: Tick::from_seconds_f64(end),
        };
        let report = trim_to_mp4(&request, &mut |_| {}, &AtomicBool::new(false))
            .unwrap_or_else(|error| panic!("{name}: {error:?}"));
        assert!(report.pictures > 0, "{name}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
