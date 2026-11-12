//! Hear the app's sound effects, or save them as WAV files to listen to anywhere.
//!
//! `cargo run -p clipcraft-audio-out --example cues` plays each one on the default output.
//! `cargo run -p clipcraft-audio-out --example cues -- --wav NEW_DIRECTORY` writes `<name>.wav`
//! files (mono, 16-bit, 48 kHz) and plays nothing.

use clipcraft_audio_out::{Cue, play_cue, render_cue};
use std::time::Duration;

const RATE: u32 = 48_000;

fn wav(samples: &[f32]) -> Vec<u8> {
    let data: Vec<u8> = samples
        .iter()
        .flat_map(|s| ((s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16).to_le_bytes())
        .collect();
    let mut out = Vec::with_capacity(44 + data.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1_u16.to_le_bytes()); // mono
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes());
    out.extend_from_slice(&2_u16.to_le_bytes());
    out.extend_from_slice(&16_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    out
}

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let folder = match (args.next().as_deref(), args.next()) {
        (Some("--wav"), Some(folder)) => Some(folder),
        (None, _) => None,
        _ => {
            eprintln!("usage: cues [--wav NEW_DIRECTORY]");
            std::process::exit(2);
        }
    };
    if let Some(folder) = &folder {
        std::fs::create_dir(folder)?;
    }
    for cue in Cue::ALL {
        let name = format!("{cue:?}");
        match &folder {
            Some(folder) => {
                std::fs::write(
                    format!("{folder}/{name}.wav"),
                    wav(&render_cue(cue, RATE, 1.0)),
                )?;
            }
            None => {
                println!("{name}");
                play_cue(cue, 100);
                std::thread::sleep(Duration::from_millis(1200));
            }
        }
    }
    Ok(())
}
