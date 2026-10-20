//! Recording what a Linux computer is playing, through the PulseAudio client library (which
//! PipeWire's PulseAudio server also answers). The "monitor" of the default output carries
//! everything sent to it.
//!
//! libpulse is loaded at run time, so a computer without it just reports that. The library's
//! simple blocking interface is used from a thread of its own that fills a bounded queue, the same
//! shape as the other sound sources. This module holds the only `unsafe` for sound.
#![allow(unsafe_code)]
#![warn(clippy::undocumented_unsafe_blocks)]

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use clipcraft_record::{AudioError, AudioFormat, AudioSource, SampleQueue};

const RATE: u32 = 48_000;
const CHANNELS: u8 = 2;
/// `PA_SAMPLE_FLOAT32LE`.
const SAMPLE_FLOAT32LE: c_int = 5;
/// `PA_STREAM_RECORD`.
const STREAM_RECORD: c_int = 2;
/// Milliseconds of sound per read.
const FRAGMENT_MS: usize = 20;
const QUEUE_SECONDS: usize = 2;

#[repr(C)]
struct SampleSpec {
    format: c_int,
    rate: u32,
    channels: u8,
}

#[repr(C)]
struct BufferAttr {
    maxlength: u32,
    tlength: u32,
    prebuf: u32,
    minreq: u32,
    fragsize: u32,
}

type NewFn = unsafe extern "C" fn(
    *const c_char,
    *const c_char,
    c_int,
    *const c_char,
    *const c_char,
    *const SampleSpec,
    *const c_void,
    *const BufferAttr,
    *mut c_int,
) -> *mut c_void;
type ReadFn = unsafe extern "C" fn(*mut c_void, *mut c_void, usize, *mut c_int) -> c_int;
type FreeFn = unsafe extern "C" fn(*mut c_void);
type ErrorFn = unsafe extern "C" fn(c_int) -> *const c_char;

/// The functions used, with the libraries that provide them kept loaded.
struct Pulse {
    new: NewFn,
    read: ReadFn,
    free: FreeFn,
    strerror: ErrorFn,
    _libraries: (libloading::Library, libloading::Library),
}

impl Pulse {
    fn load() -> Result<Self, String> {
        // SAFETY: loading the system's libpulse runs its initializers, as for any program using
        // it; the symbols are looked up by their documented names and cast to their documented C
        // signatures.
        unsafe {
            let simple = libloading::Library::new("libpulse-simple.so.0")
                .map_err(|_| "PulseAudio is not installed (libpulse-simple.so.0)".to_owned())?;
            let core = libloading::Library::new("libpulse.so.0")
                .map_err(|_| "PulseAudio is not installed (libpulse.so.0)".to_owned())?;
            let new: NewFn = *simple
                .get(b"pa_simple_new\0")
                .map_err(|_| "libpulse has no pa_simple_new".to_owned())?;
            let read: ReadFn = *simple
                .get(b"pa_simple_read\0")
                .map_err(|_| "libpulse has no pa_simple_read".to_owned())?;
            let free: FreeFn = *simple
                .get(b"pa_simple_free\0")
                .map_err(|_| "libpulse has no pa_simple_free".to_owned())?;
            let strerror: ErrorFn = *core
                .get(b"pa_strerror\0")
                .map_err(|_| "libpulse has no pa_strerror".to_owned())?;
            Ok(Self {
                new,
                read,
                free,
                strerror,
                _libraries: (simple, core),
            })
        }
    }

    fn describe(&self, code: c_int) -> String {
        // SAFETY: `pa_strerror` returns a static NUL-terminated string, or null.
        unsafe {
            let text = (self.strerror)(code);
            if text.is_null() {
                format!("error {code}")
            } else {
                CStr::from_ptr(text).to_string_lossy().into_owned()
            }
        }
    }
}

/// A `pa_simple` connection, owned by exactly one thread at a time.
struct Connection(*mut c_void);

// SAFETY: a `pa_simple` may be used from any thread as long as one thread at a time uses it; the
// connection is moved into the reading thread and never shared.
unsafe impl Send for Connection {}

/// What a PulseAudio monitor records. Dropping it stops the reading thread, which closes the
/// connection when its current read returns.
pub struct PulseMonitor {
    device: String,
    queue: Arc<SampleQueue>,
    stop: Arc<AtomicBool>,
}

/// Records everything the default output (or the named `.monitor` source) plays, as 48 kHz stereo.
pub fn open_system_monitor(device: &str) -> Result<Box<dyn AudioSource>, String> {
    PulseMonitor::open(device).map(|monitor| Box::new(monitor) as Box<dyn AudioSource>)
}

impl PulseMonitor {
    fn open(device: &str) -> Result<Self, String> {
        let source = if device.ends_with(".monitor") {
            device.to_owned()
        } else {
            "@DEFAULT_MONITOR@".to_owned()
        };
        let pulse = Arc::new(Pulse::load()?);
        let queue = Arc::new(SampleQueue::new(
            RATE as usize * usize::from(CHANNELS) * QUEUE_SECONDS,
        ));
        let stop = Arc::new(AtomicBool::new(false));
        let (reply, receive) = mpsc::channel::<Result<(), String>>();
        let thread_queue = queue.clone();
        let thread_stop = stop.clone();
        let thread_source = source.clone();
        std::thread::Builder::new()
            .name("clipcraft-pulse".to_owned())
            .spawn(move || {
                match connect(&pulse, &thread_source) {
                    Ok(connection) => {
                        let _ = reply.send(Ok(()));
                        read_loop(&pulse, connection, &thread_queue, &thread_stop);
                    }
                    Err(why) => {
                        let _ = reply.send(Err(why));
                    }
                };
            })
            .map_err(|_| "the sound thread could not start".to_owned())?;
        match receive.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(Self {
                device: source,
                queue,
                stop,
            }),
            Ok(Err(why)) => Err(why),
            Err(_) => {
                stop.store(true, Ordering::SeqCst);
                Err("PulseAudio did not answer".to_owned())
            }
        }
    }
}

fn connect(pulse: &Pulse, source: &str) -> Result<Connection, String> {
    let name = CString::new("ClipCraft").map_err(|_| "bad name".to_owned())?;
    let stream = CString::new("Game and system sound").map_err(|_| "bad name".to_owned())?;
    let device = CString::new(source).map_err(|_| "the device name is not valid".to_owned())?;
    let spec = SampleSpec {
        format: SAMPLE_FLOAT32LE,
        rate: RATE,
        channels: CHANNELS,
    };
    let fragment = (RATE as usize / 1000 * FRAGMENT_MS * usize::from(CHANNELS) * 4) as u32;
    let attr = BufferAttr {
        maxlength: u32::MAX,
        tlength: u32::MAX,
        prebuf: u32::MAX,
        minreq: u32::MAX,
        fragsize: fragment,
    };
    let mut error: c_int = 0;
    // SAFETY: every pointer refers to a live local for the duration of the call; the strings are
    // NUL-terminated; a null server and channel map ask for the defaults.
    let handle = unsafe {
        (pulse.new)(
            std::ptr::null(),
            name.as_ptr(),
            STREAM_RECORD,
            device.as_ptr(),
            stream.as_ptr(),
            &spec,
            std::ptr::null(),
            &attr,
            &mut error,
        )
    };
    if handle.is_null() {
        return Err(format!("PulseAudio: {}", pulse.describe(error)));
    }
    Ok(Connection(handle))
}

fn read_loop(pulse: &Pulse, connection: Connection, queue: &SampleQueue, stop: &AtomicBool) {
    let bytes = RATE as usize / 1000 * FRAGMENT_MS * usize::from(CHANNELS) * 4;
    let mut buffer = vec![0_u8; bytes];
    while !stop.load(Ordering::SeqCst) {
        let mut error: c_int = 0;
        // SAFETY: the connection is live and owned by this thread; `buffer` has `bytes` writable
        // bytes.
        let status =
            unsafe { (pulse.read)(connection.0, buffer.as_mut_ptr().cast(), bytes, &mut error) };
        if status < 0 {
            queue.fail();
            break;
        }
        queue.push(
            buffer
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        );
    }
    // SAFETY: the connection is freed exactly once, here, by its only owner.
    unsafe { (pulse.free)(connection.0) };
}

impl AudioSource for PulseMonitor {
    fn name(&self) -> String {
        "Game and system sound".to_owned()
    }

    fn format(&self) -> AudioFormat {
        AudioFormat {
            sample_rate: RATE,
            channels: u16::from(CHANNELS),
        }
    }

    fn read(&mut self, out: &mut Vec<f32>) -> Result<(), AudioError> {
        self.queue.drain_into(out);
        if self.queue.failed() {
            return Err(AudioError::DeviceLost("PulseAudio stopped".to_owned()));
        }
        Ok(())
    }

    fn recover(&mut self) -> bool {
        match Self::open(&self.device) {
            Ok(fresh) => {
                self.stop.store(true, Ordering::SeqCst);
                *self = fresh;
                true
            }
            Err(_) => false,
        }
    }
}

impl Drop for PulseMonitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}
