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

/// One output (a "sink") PulseAudio can play to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Output {
    /// The name PulseAudio uses; what is saved in the settings.
    pub name: String,
    /// What to call it for people.
    pub description: String,
}

/// The outputs PulseAudio has, and which one is the default.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Outputs {
    /// Every output, in the server's order.
    pub outputs: Vec<Output>,
    /// The default output's name.
    pub default: Option<String>,
}

const MAX_OUTPUTS: usize = 64;
const CONTEXT_READY: c_int = 4;
const CONTEXT_FAILED: c_int = 5;
const CONTEXT_TERMINATED: c_int = 6;
const OPERATION_RUNNING: c_int = 0;

/// The start of `pa_sink_info`; only these fields are read.
#[repr(C)]
struct SinkInfoHead {
    name: *const c_char,
    index: u32,
    description: *const c_char,
}

/// The start of `pa_server_info`; only `default_sink_name` is read.
#[repr(C)]
struct ServerInfoHead {
    user_name: *const c_char,
    host_name: *const c_char,
    server_version: *const c_char,
    server_name: *const c_char,
    sample_spec: SampleSpec,
    default_sink_name: *const c_char,
}

/// What the callbacks fill in.
struct Gathered {
    outputs: Vec<Output>,
    default: Option<String>,
}

fn text(pointer: *const c_char) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    // SAFETY: libpulse passes NUL-terminated strings that live for the callback.
    Some(
        unsafe { CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned(),
    )
}

unsafe extern "C" fn on_sink(
    _context: *mut c_void,
    info: *const SinkInfoHead,
    end_of_list: c_int,
    userdata: *mut c_void,
) {
    if end_of_list != 0 || info.is_null() || userdata.is_null() {
        return;
    }
    // SAFETY: `userdata` is the `Gathered` that `list_outputs` keeps alive while it iterates the
    // main loop, and `info` points to a sink description valid for this call.
    let (gathered, info) = unsafe { (&mut *userdata.cast::<Gathered>(), &*info) };
    if gathered.outputs.len() < MAX_OUTPUTS
        && let Some(name) = text(info.name)
    {
        let description = text(info.description).unwrap_or_else(|| name.clone());
        gathered.outputs.push(Output { name, description });
    }
}

unsafe extern "C" fn on_server(
    _context: *mut c_void,
    info: *const ServerInfoHead,
    userdata: *mut c_void,
) {
    if info.is_null() || userdata.is_null() {
        return;
    }
    // SAFETY: as in `on_sink`.
    let (gathered, info) = unsafe { (&mut *userdata.cast::<Gathered>(), &*info) };
    gathered.default = text(info.default_sink_name);
}

/// Asks PulseAudio (or PipeWire's server) which outputs exist, and which is the default.
pub fn list_outputs() -> Result<Outputs, String> {
    let lib = Introspection::load()?;
    // SAFETY: every call follows libpulse's documented lifecycle: a main loop, a context on it,
    // iterate until the context is ready, ask, iterate until the answers arrive, then release
    // the context and the loop in reverse order. Pointers come from libpulse and are not used
    // after being freed; `gathered` outlives every callback because the loop is only iterated
    // here.
    unsafe {
        let main_loop = (lib.mainloop_new)();
        if main_loop.is_null() {
            return Err("PulseAudio's main loop could not start".to_owned());
        }
        let api = (lib.mainloop_api)(main_loop);
        let name = CString::new("ClipCraft").map_err(|_| "bad name".to_owned())?;
        let context = (lib.context_new)(api, name.as_ptr());
        let mut gathered = Gathered {
            outputs: Vec::new(),
            default: None,
        };
        let result = if context.is_null() {
            Err("PulseAudio's context could not start".to_owned())
        } else {
            list_with(&lib, main_loop, context, &mut gathered)
        };
        if !context.is_null() {
            (lib.context_disconnect)(context);
            (lib.context_unref)(context);
        }
        (lib.mainloop_free)(main_loop);
        result.map(|()| Outputs {
            outputs: gathered.outputs,
            default: gathered.default,
        })
    }
}

/// Runs the main loop until `done` says so, for at most a few seconds.
///
/// # Safety
/// `main_loop` must be a live libpulse main loop.
unsafe fn spin(
    lib: &Introspection,
    main_loop: *mut c_void,
    mut done: impl FnMut() -> bool,
) -> Result<(), String> {
    let began = std::time::Instant::now();
    while !done() {
        if began.elapsed() > Duration::from_secs(5) {
            return Err("PulseAudio did not answer".to_owned());
        }
        // SAFETY: the caller guarantees the loop is live.
        let status = unsafe { (lib.mainloop_iterate)(main_loop, 0, std::ptr::null_mut()) };
        if status < 0 {
            return Err("PulseAudio's main loop stopped".to_owned());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    Ok(())
}

/// # Safety
/// `main_loop` and `context` must be live libpulse objects of the same loop.
unsafe fn list_with(
    lib: &Introspection,
    main_loop: *mut c_void,
    context: *mut c_void,
    gathered: &mut Gathered,
) -> Result<(), String> {
    // SAFETY: the caller guarantees both objects are live; the callbacks only write to
    // `gathered`, which outlives them.
    unsafe {
        if (lib.context_connect)(context, std::ptr::null(), 0, std::ptr::null()) < 0 {
            return Err("PulseAudio is not running".to_owned());
        }
        spin(lib, main_loop, || {
            matches!(
                (lib.context_state)(context),
                CONTEXT_READY | CONTEXT_FAILED | CONTEXT_TERMINATED
            )
        })?;
        if (lib.context_state)(context) != CONTEXT_READY {
            return Err("PulseAudio is not running".to_owned());
        }
        let user = (gathered as *mut Gathered).cast::<c_void>();
        let sinks = (lib.get_sinks)(context, on_sink, user);
        let server = (lib.get_server)(context, on_server, user);
        let finished = |operation: *mut c_void| {
            operation.is_null() || (lib.operation_state)(operation) != OPERATION_RUNNING
        };
        let outcome = spin(lib, main_loop, || finished(sinks) && finished(server));
        for operation in [sinks, server] {
            if !operation.is_null() {
                (lib.operation_unref)(operation);
            }
        }
        outcome
    }
}

type SinkCallback = unsafe extern "C" fn(*mut c_void, *const SinkInfoHead, c_int, *mut c_void);
type ServerCallback = unsafe extern "C" fn(*mut c_void, *const ServerInfoHead, *mut c_void);

/// The introspection functions of libpulse.
struct Introspection {
    mainloop_new: unsafe extern "C" fn() -> *mut c_void,
    mainloop_free: unsafe extern "C" fn(*mut c_void),
    mainloop_api: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    mainloop_iterate: unsafe extern "C" fn(*mut c_void, c_int, *mut c_int) -> c_int,
    context_new: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void,
    context_connect:
        unsafe extern "C" fn(*mut c_void, *const c_char, c_int, *const c_void) -> c_int,
    context_state: unsafe extern "C" fn(*mut c_void) -> c_int,
    context_disconnect: unsafe extern "C" fn(*mut c_void),
    context_unref: unsafe extern "C" fn(*mut c_void),
    get_sinks: unsafe extern "C" fn(*mut c_void, SinkCallback, *mut c_void) -> *mut c_void,
    get_server: unsafe extern "C" fn(*mut c_void, ServerCallback, *mut c_void) -> *mut c_void,
    operation_state: unsafe extern "C" fn(*mut c_void) -> c_int,
    operation_unref: unsafe extern "C" fn(*mut c_void),
    _library: libloading::Library,
}

impl Introspection {
    fn load() -> Result<Self, String> {
        // SAFETY: as in `Pulse::load`: documented names cast to documented C signatures.
        unsafe {
            let library = libloading::Library::new("libpulse.so.0")
                .map_err(|_| "PulseAudio is not installed (libpulse.so.0)".to_owned())?;
            macro_rules! symbol {
                ($name:literal) => {
                    *library
                        .get(concat!($name, "\0").as_bytes())
                        .map_err(|_| format!("libpulse has no {}", $name))?
                };
            }
            Ok(Self {
                mainloop_new: symbol!("pa_mainloop_new"),
                mainloop_free: symbol!("pa_mainloop_free"),
                mainloop_api: symbol!("pa_mainloop_get_api"),
                mainloop_iterate: symbol!("pa_mainloop_iterate"),
                context_new: symbol!("pa_context_new"),
                context_connect: symbol!("pa_context_connect"),
                context_state: symbol!("pa_context_get_state"),
                context_disconnect: symbol!("pa_context_disconnect"),
                context_unref: symbol!("pa_context_unref"),
                get_sinks: symbol!("pa_context_get_sink_info_list"),
                get_server: symbol!("pa_context_get_server_info"),
                operation_state: symbol!("pa_operation_get_state"),
                operation_unref: symbol!("pa_operation_unref"),
                _library: library,
            })
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
        // The chosen output's monitor; the default output's when none is chosen or the choice is
        // not an output PulseAudio knows (a name saved from another sound system).
        let source = if device.ends_with(".monitor") {
            device.to_owned()
        } else if !device.is_empty()
            && list_outputs()
                .is_ok_and(|found| found.outputs.iter().any(|sink| sink.name == device))
        {
            format!("{device}.monitor")
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
