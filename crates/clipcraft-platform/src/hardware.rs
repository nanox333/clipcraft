//! Hardware H.264 encoding, through FilmCraft's platform crate: NVIDIA NVENC on Windows and
//! 64-bit Linux, VideoToolbox on macOS. Other graphics cards have no encoder yet; the recorder
//! then uses the software encoder and says why.
//!
//! FilmCraft's encoders take the export pipeline's types, so this module translates: it builds
//! the export settings a recording needs, hands each picture over as RGBA, and turns the packets
//! back into the recorder's own. B-frames are switched off by using the Baseline profile, and any
//! reordered packet is reported so the recorder can fall back.

use clipcraft_record::{HardwareEncoder, HardwarePacket, HardwareProvider, HardwareRequest};
use filmcraft_export::{
    BitrateMode, EncoderFactory, EncoderFrame, ExportSettings, Format, H264Profile,
    HardwareEncoding, VideoEncoder,
};
use filmcraft_isobmff::CodecConfig;
use filmcraft_time::FrameRate;

/// What this system's hardware encoder is called, and the function that opens it.
fn backend() -> Option<(&'static str, EncoderFactory)> {
    #[cfg(any(windows, all(target_os = "linux", target_pointer_width = "64")))]
    {
        Some(("NVIDIA NVENC", filmcraft_platform::nvenc::export::factory))
    }
    #[cfg(target_os = "macos")]
    {
        Some((
            "Apple VideoToolbox",
            filmcraft_platform::hardware_encode::videotoolbox_encoder_factory,
        ))
    }
    #[cfg(not(any(
        windows,
        all(target_os = "linux", target_pointer_width = "64"),
        target_os = "macos"
    )))]
    {
        None
    }
}

struct Wrapped {
    encoder: Box<dyn VideoEncoder>,
    name: &'static str,
    width: u32,
    height: u32,
}

fn packets(
    output: Result<Vec<filmcraft_export::EncodedPacket>, filmcraft_export::ExportError>,
) -> Result<Vec<HardwarePacket>, String> {
    Ok(output
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|packet| HardwarePacket {
            data: packet.data,
            keyframe: packet.key,
            reordered: packet.composition_offset != 0,
        })
        .collect())
}

impl HardwareEncoder for Wrapped {
    fn encode(&mut self, rgba: &[u8], index: u64) -> Result<Vec<HardwarePacket>, String> {
        // The recorder keeps every picture at the size the encoder was opened with.
        let (width, height) = (self.width, self.height);
        packets(self.encoder.encode(&EncoderFrame {
            width,
            height,
            rgba,
            hdr: None,
            index,
        }))
    }

    fn flush(&mut self) -> Result<Vec<HardwarePacket>, String> {
        packets(self.encoder.flush())
    }

    fn avcc(&self) -> Vec<u8> {
        match self.encoder.sample_entry().codec {
            CodecConfig::Avc(config) => config.to_bytes(),
            _ => Vec::new(),
        }
    }

    fn name(&self) -> String {
        self.name.to_owned()
    }
}

fn open(request: &HardwareRequest) -> Result<Box<dyn HardwareEncoder>, String> {
    let Some((name, factory)) = backend() else {
        return Err("this computer has no supported hardware encoder".to_owned());
    };
    let settings = ExportSettings {
        format: Format::H264,
        hardware_encoding: HardwareEncoding::Auto,
        bitrate_kbps: request.video_kbps,
        bitrate_mode: if request.constant_bitrate {
            BitrateMode::Cbr
        } else {
            BitrateMode::Vbr1Pass
        },
        keyframe_distance: Some(request.keyframe_frames),
        // The Baseline profile has no B-frames, so pictures come out in the order they go in.
        h264_profile: H264Profile::Baseline,
        ..ExportSettings::default()
    };
    let rate = FrameRate {
        num: i64::from(request.fps),
        den: 1,
    };
    match factory(Format::H264, request.width, request.height, rate, &settings) {
        None => Err(
            "no NVIDIA or Apple hardware encoder accepted this recording (other graphics cards are not supported yet)"
                .to_owned(),
        ),
        Some(Err(error)) => Err(error.to_string()),
        Some(Ok(encoder)) => Ok(Box::new(Wrapped {
            encoder,
            name,
            width: request.width,
            height: request.height,
        })),
    }
}

/// Opens hardware encoders for the recorder, or `None` on a system with no hardware encoder code.
pub fn hardware_provider() -> Option<HardwareProvider> {
    backend().map(|_| HardwareProvider::new(open))
}

/// Tries a small hardware encoder to learn whether the GPU can be used. Returns its name, or the
/// reason it cannot. Takes a fraction of a second; call it off the interface thread.
pub fn probe_hardware_encoder() -> Result<String, String> {
    let provider = hardware_provider()
        .ok_or_else(|| "this computer has no supported hardware encoder".to_owned())?;
    let encoder = provider.open(&HardwareRequest {
        width: 256,
        height: 144,
        fps: 30,
        video_kbps: 1_000,
        constant_bitrate: true,
        keyframe_frames: 30,
    })?;
    Ok(encoder.name())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_computer_without_the_hardware_says_why_instead_of_failing() {
        // Neither the NVIDIA driver nor a GPU exists in the test environment.
        match probe_hardware_encoder() {
            Ok(name) => assert!(!name.is_empty()),
            Err(why) => {
                println!("probe said: {why}");
                assert!(!why.is_empty());
            }
        }
    }

    #[test]
    fn opening_with_nonsense_is_an_error_not_a_panic() {
        let Some(provider) = hardware_provider() else {
            return;
        };
        for (width, height) in [(0, 0), (1, 1), (3, 5), (100_000, 100_000)] {
            let result = provider.open(&HardwareRequest {
                width,
                height,
                fps: 30,
                video_kbps: 1_000,
                constant_bitrate: false,
                keyframe_frames: 0,
            });
            assert!(result.is_err(), "{width}x{height}");
        }
    }
}
