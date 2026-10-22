//! Hardware H.264 encoding, through FilmCraft's platform crate: NVIDIA NVENC on Windows and
//! 64-bit Linux, VideoToolbox on macOS; AMD and Intel through Media Foundation on Windows and
//! VA-API on Linux. When none works, the recorder uses the software encoder and says why.
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

/// FilmCraft's encoders: NVIDIA on Windows and Linux, Apple on macOS.
fn open_filmcraft(request: &HardwareRequest) -> Result<Box<dyn HardwareEncoder>, String> {
    let Some((name, factory)) = backend() else {
        return Err("this computer has no NVIDIA or Apple hardware encoder code".to_owned());
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
        None => Err(format!("no {name} encoder accepted this recording")),
        Some(Err(error)) => Err(error.to_string()),
        Some(Ok(encoder)) => Ok(Box::new(Wrapped {
            encoder,
            name,
            width: request.width,
            height: request.height,
        })),
    }
}

/// Puts the operating system's hardware video decoders (NVDEC, VA-API, Media Foundation,
/// VideoToolbox) in front of FilmCraft's software ones, switched off. Nothing changes for playback;
/// an export that is allowed to use the GPU switches them on while it opens its decoder. A stream
/// the hardware cannot take, or a decoder that fails part-way, falls back to software by itself.
/// Returns the backend's name, or `None` when this computer has no hardware decoder.
pub fn register_hardware_decoding() -> Option<&'static str> {
    use std::sync::OnceLock;
    static BACKEND: OnceLock<Option<&'static str>> = OnceLock::new();
    *BACKEND.get_or_init(|| {
        let backend = match filmcraft_platform::register() {
            filmcraft_platform::Availability::Available(name) => Some(name),
            _ => None,
        };
        filmcraft_codecs::hw::set_hardware_decoding(false);
        backend
    })
}

/// Tries each hardware encoder this system has, in turn. On Windows that is NVIDIA's (through
/// FilmCraft) and then whatever hardware H.264 encoder the graphics driver installs for Media
/// Foundation, which is how AMD and Intel cards are reached. On Linux it is NVIDIA's and then
/// VA-API, which is how AMD (Mesa) and Intel cards are reached.
fn open(request: &HardwareRequest) -> Result<Box<dyn HardwareEncoder>, String> {
    let mut reasons = Vec::new();
    match open_filmcraft(request) {
        Ok(encoder) => return Ok(encoder),
        Err(why) => reasons.push(why),
    }
    #[cfg(windows)]
    match crate::mf_encoder::open(request) {
        Ok(encoder) => return Ok(encoder),
        Err(why) => reasons.push(why),
    }
    #[cfg(target_os = "linux")]
    match crate::vaapi::encoder::open(request) {
        Ok(encoder) => return Ok(encoder),
        Err(why) => reasons.push(why),
    }
    Err(reasons.join("; "))
}

/// Opens hardware encoders for the recorder, or `None` on a system with no hardware encoder code.
pub fn hardware_provider() -> Option<HardwareProvider> {
    let available = backend().is_some() || cfg!(windows) || cfg!(target_os = "linux");
    available.then(|| HardwareProvider::new(open))
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
