//! Plays a clip's sound through the default output device with no window: a quick way to check
//! that sound works on a machine.
//!
//! `cargo run -p clipcraft-audio-out --example play -- /path/to/video.mp4`

use clipcraft_audio_out::AudioOutput;
use clipcraft_player::{Player, PlayerEvent};
use std::time::{Duration, Instant};

fn main() {
    let Some(path) = std::env::args_os().nth(1) else {
        eprintln!("usage: play <video file>");
        std::process::exit(2);
    };
    let player = Player::start(path.into(), || {});
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut output = None;
    let mut frames = 0_u32;
    while Instant::now() < deadline {
        for event in player.poll() {
            match event {
                PlayerEvent::Opened(info) => {
                    println!(
                        "opened: {}x{}, {:.2} s, sound: {}",
                        info.width,
                        info.height,
                        info.duration.seconds(),
                        info.has_audio
                    );
                    if let Some(feed) = player.audio_feed() {
                        match AudioOutput::start(feed) {
                            Ok(started) => {
                                let (rate, channels) = started.format();
                                println!("sound device started: {rate} Hz, {channels} channels");
                                output = Some(started);
                            }
                            Err(error) => println!("no sound: {error}"),
                        }
                    }
                    player.play();
                }
                PlayerEvent::Frame(_) => frames += 1,
                PlayerEvent::Ended => {
                    println!("ended after {frames} pictures");
                    return;
                }
                PlayerEvent::Failed(error) => {
                    println!("failed: {error}");
                    return;
                }
            }
        }
        if output.as_ref().is_some_and(AudioOutput::failed) {
            println!("sound device reported an error");
            output = None;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    println!("timed out");
}
