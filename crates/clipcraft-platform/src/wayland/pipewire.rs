//! Receiving the pictures of a screen share from PipeWire.
//!
//! libpipewire is loaded at run time, so a computer without it just reports that. A thread of its
//! own runs PipeWire's main loop, owns every PipeWire object, and leaves the newest picture in a
//! shared slot for [`Stream::next_frame`]. This module holds the only `unsafe` for Wayland; the C
//! structure layouts it reads are checked against the headers in the tests (see docs/wayland.md).
#![allow(unsafe_code)]
#![warn(clippy::undocumented_unsafe_blocks)]

use std::ffi::{CStr, c_char, c_int, c_void};
use std::os::fd::{IntoRawFd, OwnedFd};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use clipcraft_capture::{CaptureError, MAX_CAPTURE_EDGE};

use super::pod::{self, Negotiated, PixelFormat};

/// `PW_DIRECTION_INPUT`: this program receives.
const DIRECTION_INPUT: c_int = 0;
/// `PW_STREAM_FLAG_AUTOCONNECT`: link to the node given, and `PW_STREAM_FLAG_MAP_BUFFERS`: have
/// PipeWire map shared memory so the picture is readable.
const FLAGS: u32 = 1 | 4;
/// `PW_ID_ANY`.
const ID_ANY: u32 = u32::MAX;
/// `PW_STREAM_STATE_ERROR`.
const STATE_ERROR: c_int = -1;
/// `PW_STREAM_STATE_PAUSED` and `PW_STREAM_STATE_STREAMING`.
const STATE_PAUSED: c_int = 2;
const STATE_STREAMING: c_int = 3;
/// A stream that was sending and has been paused this long is taken to be over: when the thing
/// being shared goes away, PipeWire pauses the stream rather than reporting an error.
const PAUSED_MEANS_GONE: Duration = Duration::from_secs(5);
/// `PW_STREAM_STATE_UNCONNECTED`.
const STATE_UNCONNECTED: c_int = 0;
/// `SPA_PARAM_Format`.
const PARAM_FORMAT: u32 = 4;
/// `SPA_CHUNK_FLAG_CORRUPTED`.
const CHUNK_CORRUPTED: i32 = 1;
/// Most bytes a single picture may take (an 8K frame).
const MAX_FRAME_BYTES: usize = MAX_CAPTURE_EDGE as usize * MAX_CAPTURE_EDGE as usize * 4;

#[repr(C)]
struct SpaChunk {
    offset: u32,
    size: u32,
    stride: i32,
    flags: i32,
}

#[repr(C)]
struct SpaData {
    kind: u32,
    flags: u32,
    fd: i64,
    map_offset: u32,
    max_size: u32,
    data: *mut c_void,
    chunk: *mut SpaChunk,
}

#[repr(C)]
struct SpaBuffer {
    n_metas: u32,
    n_datas: u32,
    metas: *mut c_void,
    datas: *mut SpaData,
}

/// Only the first field of `pw_buffer` is used.
#[repr(C)]
struct PwBuffer {
    buffer: *mut SpaBuffer,
}

type StateChanged = unsafe extern "C" fn(*mut c_void, c_int, c_int, *const c_char);
type ParamChanged = unsafe extern "C" fn(*mut c_void, u32, *const c_void);
type Process = unsafe extern "C" fn(*mut c_void);

/// `struct pw_stream_events` (version 2).
#[repr(C)]
struct StreamEvents {
    version: u32,
    destroy: Option<unsafe extern "C" fn(*mut c_void)>,
    state_changed: Option<StateChanged>,
    control_info: Option<unsafe extern "C" fn()>,
    io_changed: Option<unsafe extern "C" fn()>,
    param_changed: Option<ParamChanged>,
    add_buffer: Option<unsafe extern "C" fn()>,
    remove_buffer: Option<unsafe extern "C" fn()>,
    process: Option<Process>,
    drained: Option<unsafe extern "C" fn()>,
    command: Option<unsafe extern "C" fn()>,
    trigger_done: Option<unsafe extern "C" fn()>,
}

/// `struct spa_hook` is 48 bytes; PipeWire fills it in.
type Hook = [u64; 6];

/// The functions used, with the library that provides them kept loaded.
#[derive(Clone, Copy)]
struct Api {
    init: unsafe extern "C" fn(*mut c_int, *mut c_void),
    loop_new: unsafe extern "C" fn(*const c_void) -> *mut c_void,
    loop_get: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    loop_run: unsafe extern "C" fn(*mut c_void) -> c_int,
    loop_quit: unsafe extern "C" fn(*mut c_void) -> c_int,
    loop_destroy: unsafe extern "C" fn(*mut c_void),
    context_new: unsafe extern "C" fn(*mut c_void, *mut c_void, usize) -> *mut c_void,
    context_connect_fd: unsafe extern "C" fn(*mut c_void, c_int, *mut c_void, usize) -> *mut c_void,
    context_destroy: unsafe extern "C" fn(*mut c_void),
    core_disconnect: unsafe extern "C" fn(*mut c_void) -> c_int,
    properties_new: unsafe extern "C" fn(*const c_char, ...) -> *mut c_void,
    stream_new: unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_void) -> *mut c_void,
    stream_add_listener:
        unsafe extern "C" fn(*mut c_void, *mut Hook, *const StreamEvents, *mut c_void),
    stream_connect:
        unsafe extern "C" fn(*mut c_void, c_int, u32, u32, *mut *const c_void, u32) -> c_int,
    stream_update_params: unsafe extern "C" fn(*mut c_void, *mut *const c_void, u32) -> c_int,
    stream_dequeue: unsafe extern "C" fn(*mut c_void) -> *mut PwBuffer,
    stream_queue: unsafe extern "C" fn(*mut c_void, *mut PwBuffer) -> c_int,
    stream_destroy: unsafe extern "C" fn(*mut c_void),
}

struct Loaded {
    api: Api,
    // Keeps the functions above valid.
    _library: libloading::Library,
}

fn load() -> Option<Loaded> {
    // SAFETY: loading libpipewire runs its initialisers, which only set up its own state.
    let library = unsafe {
        libloading::Library::new("libpipewire-0.3.so.0")
            .or_else(|_| libloading::Library::new("libpipewire-0.3.so"))
    }
    .ok()?;
    macro_rules! symbol {
        ($name:literal) => {
            // SAFETY: the symbol has the C signature of the field it is stored in (checked
            // against pipewire/*.h); the function pointer is copied out and the library stays
            // loaded for as long as the copy can be used.
            *unsafe { library.get($name) }.ok()?
        };
    }
    let api = Api {
        init: symbol!(b"pw_init\0"),
        loop_new: symbol!(b"pw_main_loop_new\0"),
        loop_get: symbol!(b"pw_main_loop_get_loop\0"),
        loop_run: symbol!(b"pw_main_loop_run\0"),
        loop_quit: symbol!(b"pw_main_loop_quit\0"),
        loop_destroy: symbol!(b"pw_main_loop_destroy\0"),
        context_new: symbol!(b"pw_context_new\0"),
        context_connect_fd: symbol!(b"pw_context_connect_fd\0"),
        context_destroy: symbol!(b"pw_context_destroy\0"),
        core_disconnect: symbol!(b"pw_core_disconnect\0"),
        properties_new: symbol!(b"pw_properties_new\0"),
        stream_new: symbol!(b"pw_stream_new\0"),
        stream_add_listener: symbol!(b"pw_stream_add_listener\0"),
        stream_connect: symbol!(b"pw_stream_connect\0"),
        stream_update_params: symbol!(b"pw_stream_update_params\0"),
        stream_dequeue: symbol!(b"pw_stream_dequeue_buffer\0"),
        stream_queue: symbol!(b"pw_stream_queue_buffer\0"),
        stream_destroy: symbol!(b"pw_stream_destroy\0"),
    };
    Some(Loaded {
        api,
        _library: library,
    })
}

/// Whether PipeWire's client library can be loaded here.
pub fn available() -> bool {
    load().is_some()
}

/// One picture: tightly packed BGRA, `width * 4` bytes per row.
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

#[derive(Default)]
struct State {
    negotiated: Option<Negotiated>,
    picture: Option<Picture>,
    /// Counts pictures received, so a reader can tell a new one from the one it has.
    sequence: u64,
    error: Option<CaptureError>,
    /// Whether any picture has been sent, and since when the stream has been paused after that.
    was_streaming: bool,
    paused_since: Option<Instant>,
    /// The main loop, while it exists, for [`Stream`]'s drop to stop it.
    main_loop: usize,
    loop_returned: bool,
    closing: bool,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// What the callbacks get back as their `data` pointer.
struct Callbacks {
    shared: Arc<Shared>,
    api: Api,
    stream: *mut c_void,
    events: StreamEvents,
    hook: Hook,
}

fn fail(shared: &Shared, error: CaptureError) {
    let mut state = shared.lock();
    state.error.get_or_insert(error);
    drop(state);
    shared.changed.notify_all();
}

/// # Safety
/// `data` must be the `Callbacks` pointer given to `stream_add_listener`, and `error` null or a
/// C string.
unsafe extern "C" fn on_state(data: *mut c_void, _old: c_int, state: c_int, error: *const c_char) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `data` is the live `Callbacks` box (it outlives the stream; see `run`).
        let Some(callbacks) = (unsafe { data.cast::<Callbacks>().as_ref() }) else {
            return;
        };
        if state == STATE_ERROR {
            let _detail = if error.is_null() {
                None
            } else {
                // SAFETY: PipeWire passes a NUL-terminated message for the call's duration.
                Some(unsafe { CStr::from_ptr(error) })
            };
            fail(&callbacks.shared, CaptureError::Failed);
        } else if state == STATE_STREAMING {
            let mut guard = callbacks.shared.lock();
            guard.was_streaming = true;
            guard.paused_since = None;
        } else if state == STATE_PAUSED {
            let mut guard = callbacks.shared.lock();
            if guard.was_streaming {
                guard.paused_since.get_or_insert_with(Instant::now);
            }
        } else if state == STATE_UNCONNECTED {
            // The share ended: the person stopped it, or the screen went away.
            fail(&callbacks.shared, CaptureError::TargetGone);
        }
    }));
}

/// # Safety
/// `data` must be the `Callbacks` pointer, and `param` null or a pod that is valid for
/// `size + 8` bytes.
unsafe extern "C" fn on_param(data: *mut c_void, id: u32, param: *const c_void) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if id != PARAM_FORMAT || param.is_null() {
            return;
        }
        // SAFETY: `data` is the live `Callbacks` box.
        let Some(callbacks) = (unsafe { data.cast::<Callbacks>().as_ref() }) else {
            return;
        };
        // SAFETY: a pod starts with its 32-bit body size, and is followed by that many bytes
        // plus the 8-byte header.
        let size = unsafe { param.cast::<u32>().read_unaligned() } as usize;
        let Some(total) = size.checked_add(8).filter(|total| *total <= 1 << 16) else {
            return;
        };
        // SAFETY: see above; `total` is bounded.
        let bytes = unsafe { std::slice::from_raw_parts(param.cast::<u8>(), total) };
        let negotiated = pod::parse_format(bytes);
        let mut state = callbacks.shared.lock();
        state.negotiated = negotiated;
        drop(state);
        let Some(negotiated) = negotiated else {
            fail(&callbacks.shared, CaptureError::Failed);
            return;
        };
        if !negotiated.fixed {
            // The other end left choices open; pick its defaults.
            let answer = aligned(&pod::fixed_format(&negotiated));
            let mut params: [*const c_void; 1] = [answer.as_ptr().cast()];
            // SAFETY: the stream is live while its callbacks run, and `params` holds one valid,
            // aligned pod that PipeWire copies before returning.
            unsafe {
                (callbacks.api.stream_update_params)(callbacks.stream, params.as_mut_ptr(), 1)
            };
        }
    }));
}

/// Pods are read in place, so they must start on an 8-byte boundary.
fn aligned(bytes: &[u8]) -> Vec<u64> {
    let mut words = vec![0_u64; bytes.len().div_ceil(8)];
    for (word, chunk) in words.iter_mut().zip(bytes.chunks(8)) {
        let mut padded = [0_u8; 8];
        if let Some(target) = padded.get_mut(..chunk.len()) {
            target.copy_from_slice(chunk);
        }
        *word = u64::from_ne_bytes(padded);
    }
    words
}

/// Turns tightly packed pixels in `format` into opaque BGRA.
fn to_bgra(mut packed: Vec<u8>, format: PixelFormat) -> Vec<u8> {
    for pixel in packed.chunks_exact_mut(4) {
        if format.red_first()
            && let [first, _, third, _] = pixel
        {
            std::mem::swap(first, third);
        }
        // Pictures of a screen are opaque; the fourth byte is padding or unreliable.
        if let Some(alpha) = pixel.get_mut(3) {
            *alpha = 255;
        }
    }
    packed
}

/// # Safety
/// `data` must be the `Callbacks` pointer; the stream must be live.
unsafe extern "C" fn on_process(data: *mut c_void) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `data` is the live `Callbacks` box.
        let Some(callbacks) = (unsafe { data.cast::<Callbacks>().as_ref() }) else {
            return;
        };
        let api = callbacks.api;
        // Take the newest buffer and hand older ones straight back.
        let mut newest: *mut PwBuffer = std::ptr::null_mut();
        loop {
            // SAFETY: the stream is live while its callbacks run.
            let buffer = unsafe { (api.stream_dequeue)(callbacks.stream) };
            if buffer.is_null() {
                break;
            }
            if !newest.is_null() {
                // SAFETY: `newest` came from dequeue on this stream and is not used again.
                unsafe { (api.stream_queue)(callbacks.stream, newest) };
            }
            newest = buffer;
        }
        if newest.is_null() {
            return;
        }
        let picture = {
            let negotiated = callbacks.shared.lock().negotiated;
            negotiated.and_then(|negotiated| {
                // SAFETY: `newest` is a buffer PipeWire lent us until it is queued back below.
                unsafe { read_picture(newest, negotiated) }
            })
        };
        // SAFETY: `newest` came from dequeue on this stream and is not used after this.
        unsafe { (api.stream_queue)(callbacks.stream, newest) };
        if let Some(picture) = picture {
            let mut state = callbacks.shared.lock();
            state.picture = Some(picture);
            state.sequence = state.sequence.wrapping_add(1);
            drop(state);
            callbacks.shared.changed.notify_all();
        }
    }));
}

/// Copies the picture out of a lent buffer, or `None` when it is empty, damaged, or not in
/// memory this process can read (for example a GPU buffer).
///
/// # Safety
/// `buffer` must be a buffer dequeued from a live stream and not yet queued back.
unsafe fn read_picture(buffer: *mut PwBuffer, negotiated: Negotiated) -> Option<Picture> {
    // SAFETY: a dequeued `pw_buffer` is valid and its first field is the `spa_buffer`.
    let spa = unsafe { buffer.as_ref()?.buffer.as_ref()? };
    if spa.n_datas == 0 || spa.datas.is_null() {
        return None;
    }
    // SAFETY: `datas` points at `n_datas >= 1` entries.
    let data = unsafe { spa.datas.as_ref()? };
    // SAFETY: the chunk, when present, is valid for the buffer's lifetime.
    let chunk = unsafe { data.chunk.as_ref()? };
    if data.data.is_null() || chunk.flags & CHUNK_CORRUPTED != 0 {
        return None;
    }
    let width = negotiated.width as usize;
    let height = negotiated.height as usize;
    let row = width.checked_mul(4)?;
    let stride = if chunk.stride > 0 {
        chunk.stride as usize
    } else {
        row
    };
    let total = row.checked_mul(height)?;
    if width > MAX_CAPTURE_EDGE as usize || height > MAX_CAPTURE_EDGE as usize {
        return None;
    }
    if total > MAX_FRAME_BYTES || stride < row {
        return None;
    }
    let offset = chunk.offset as usize;
    let needed = stride.checked_mul(height - 1)?.checked_add(row)?;
    let end = offset.checked_add(needed)?;
    if end > data.max_size as usize || (chunk.size as usize) < needed {
        return None;
    }
    let mut packed = vec![0_u8; total];
    for index in 0..height {
        let from = offset + index * stride;
        // SAFETY: `from + row <= end <= max_size`, inside the mapped buffer; the destination
        // row `index * row .. + row` is inside `packed`. The two cannot overlap.
        unsafe {
            std::ptr::copy_nonoverlapping(
                data.data.cast::<u8>().add(from),
                packed.as_mut_ptr().add(index * row),
                row,
            );
        }
    }
    Some(Picture {
        width: negotiated.width,
        height: negotiated.height,
        bgra: to_bgra(packed, negotiated.format),
    })
}

/// Runs PipeWire's loop for one share until asked to stop, then tears everything down.
fn run(api: Api, fd: OwnedFd, node: u32, shared: &Arc<Shared>) {
    // SAFETY: every call below follows the PipeWire API contract: each object is created from
    // the one before it, used only from this thread, and destroyed in reverse order.
    unsafe {
        (api.init)(std::ptr::null_mut(), std::ptr::null_mut());
        let main_loop = (api.loop_new)(std::ptr::null());
        if main_loop.is_null() {
            fail(shared, CaptureError::Failed);
        } else {
            shared.lock().main_loop = main_loop as usize;
            session(api, fd, node, shared, main_loop);
        }
        // The loop pointer stays valid until the owner asks us to close, so a stop request
        // never reaches freed memory.
        let mut state = shared.lock();
        state.loop_returned = true;
        shared.changed.notify_all();
        while !state.closing {
            state = shared
                .changed
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        state.main_loop = 0;
        drop(state);
        if !main_loop.is_null() {
            (api.loop_destroy)(main_loop);
        }
    }
}

/// # Safety
/// `main_loop` must be a live loop used only from this thread.
unsafe fn session(api: Api, fd: OwnedFd, node: u32, shared: &Arc<Shared>, main_loop: *mut c_void) {
    // SAFETY: the loop is live; the context borrows it.
    let context = unsafe { (api.context_new)((api.loop_get)(main_loop), std::ptr::null_mut(), 0) };
    if context.is_null() {
        fail(shared, CaptureError::Failed);
        return;
    }
    // SAFETY: on success PipeWire takes over the descriptor, so it is released from `OwnedFd`.
    let core =
        unsafe { (api.context_connect_fd)(context, fd.into_raw_fd(), std::ptr::null_mut(), 0) };
    if core.is_null() {
        fail(shared, CaptureError::Denied);
        // SAFETY: the context is live and has no children.
        unsafe { (api.context_destroy)(context) };
        return;
    }
    // SAFETY: NUL-terminated strings and a NULL-terminated variadic list, as the function needs.
    let properties = unsafe {
        (api.properties_new)(
            c"media.type".as_ptr(),
            c"Video".as_ptr(),
            c"media.category".as_ptr(),
            c"Capture".as_ptr(),
            c"media.role".as_ptr(),
            c"Screen".as_ptr(),
            std::ptr::null::<c_char>(),
        )
    };
    // SAFETY: `core` is live; the stream takes over `properties`.
    let stream = unsafe { (api.stream_new)(core, c"ClipCraft".as_ptr(), properties) };
    if stream.is_null() {
        fail(shared, CaptureError::Failed);
    } else {
        let callbacks = Box::into_raw(Box::new(Callbacks {
            shared: Arc::clone(shared),
            api,
            stream,
            events: StreamEvents {
                version: 2,
                destroy: None,
                state_changed: Some(on_state),
                control_info: None,
                io_changed: None,
                param_changed: Some(on_param),
                add_buffer: None,
                remove_buffer: None,
                process: Some(on_process),
                drained: None,
                command: None,
                trigger_done: None,
            },
            hook: [0; 6],
        }));
        let offered = pod::enum_format(MAX_CAPTURE_EDGE);
        let offered = aligned(&offered);
        let mut params: [*const c_void; 1] = [offered.as_ptr().cast()];
        // SAFETY: `callbacks` is a live allocation freed only after the stream is destroyed;
        // the events table and hook live inside it. `params` holds one valid, aligned pod.
        let connected = unsafe {
            (api.stream_add_listener)(
                stream,
                &raw mut (*callbacks).hook,
                &raw const (*callbacks).events,
                callbacks.cast(),
            );
            (api.stream_connect)(
                stream,
                DIRECTION_INPUT,
                if node == 0 { ID_ANY } else { node },
                FLAGS,
                params.as_mut_ptr(),
                1,
            )
        };
        if connected < 0 {
            fail(shared, CaptureError::Failed);
        } else {
            // SAFETY: the loop is live and has not been quit.
            unsafe { (api.loop_run)(main_loop) };
        }
        // SAFETY: destroying the stream stops its callbacks, after which nothing refers to the
        // box.
        unsafe {
            (api.stream_destroy)(stream);
            drop(Box::from_raw(callbacks));
        }
    }
    // SAFETY: `core` and `context` are live and no longer referenced.
    unsafe {
        (api.core_disconnect)(core);
        (api.context_destroy)(context);
    }
}

/// A running screen stream.
pub struct Stream {
    shared: Arc<Shared>,
    api: Api,
    thread: Option<JoinHandle<()>>,
    _library: libloading::Library,
    seen: u64,
}

impl Stream {
    /// Connects to the PipeWire node `node` through `fd` and waits until the first picture
    /// arrives.
    pub fn open(fd: OwnedFd, node: u32, wait: Duration) -> Result<Self, CaptureError> {
        let Loaded { api, _library } = load().ok_or(CaptureError::NoProvider)?;
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
        });
        let worker = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("clipcraft-pipewire".to_owned())
            .spawn(move || {
                let _ = catch_unwind(AssertUnwindSafe(|| run(api, fd, node, &worker)));
            })
            .map_err(|_| CaptureError::Failed)?;
        let mut stream = Self {
            shared,
            api,
            thread: Some(thread),
            _library,
            seen: 0,
        };
        stream.wait_for_picture(wait)?;
        Ok(stream)
    }

    fn wait_for_picture(&mut self, wait: Duration) -> Result<(), CaptureError> {
        let deadline = Instant::now() + wait;
        let mut state = self.shared.lock();
        loop {
            if let Some(error) = state.error {
                return Err(error);
            }
            if state.picture.is_some() {
                return Ok(());
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(CaptureError::Failed);
            }
            state = self
                .shared
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
    }

    /// The newest picture. Waits up to `wait` for one newer than the last returned; when none
    /// comes (a still screen sends nothing) the second value is `false` and the picture is the
    /// previous one again.
    pub fn next_picture(&mut self, wait: Duration) -> Result<(Picture, bool), CaptureError> {
        let deadline = Instant::now() + wait;
        let mut state = self.shared.lock();
        loop {
            if let Some(error) = state.error {
                return Err(error);
            }
            if state
                .paused_since
                .is_some_and(|since| since.elapsed() > PAUSED_MEANS_GONE)
            {
                state.error = Some(CaptureError::TargetGone);
                return Err(CaptureError::TargetGone);
            }
            if state.sequence != self.seen
                && let Some(picture) = state.picture.as_ref()
            {
                self.seen = state.sequence;
                return Ok((copy(picture), true));
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return state
                    .picture
                    .as_ref()
                    .map(|picture| (copy(picture), false))
                    .ok_or(CaptureError::Failed);
            }
            state = self
                .shared
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
    }
}

fn copy(picture: &Picture) -> Picture {
    Picture {
        width: picture.width,
        height: picture.height,
        bgra: picture.bgra.clone(),
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        // A stop that arrives before the loop starts running is lost, so ask until it has
        // returned.
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut state = self.shared.lock();
        while !state.loop_returned && Instant::now() < deadline {
            if state.main_loop != 0 {
                // SAFETY: the loop thread keeps the loop alive until `closing` is set below.
                unsafe { (self.api.loop_quit)(state.main_loop as *mut c_void) };
            }
            state = self
                .shared
                .changed
                .wait_timeout(state, Duration::from_millis(20))
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
        state.closing = true;
        drop(state);
        self.shared.changed.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    /// Layouts measured with a C compiler against the PipeWire 0.3 headers.
    #[test]
    fn the_structures_read_from_pipewire_match_its_headers() {
        assert_eq!(size_of::<Hook>(), 48);
        assert_eq!(size_of::<StreamEvents>(), 96);
        assert_eq!(offset_of!(StreamEvents, state_changed), 16);
        assert_eq!(offset_of!(StreamEvents, param_changed), 40);
        assert_eq!(offset_of!(StreamEvents, process), 64);
        assert_eq!(size_of::<SpaBuffer>(), 24);
        assert_eq!(offset_of!(SpaBuffer, n_datas), 4);
        assert_eq!(offset_of!(SpaBuffer, datas), 16);
        assert_eq!(size_of::<SpaData>(), 40);
        assert_eq!(offset_of!(SpaData, fd), 8);
        assert_eq!(offset_of!(SpaData, max_size), 20);
        assert_eq!(offset_of!(SpaData, data), 24);
        assert_eq!(offset_of!(SpaData, chunk), 32);
        assert_eq!(size_of::<SpaChunk>(), 16);
        assert_eq!(offset_of!(SpaChunk, size), 4);
        assert_eq!(offset_of!(SpaChunk, stride), 8);
        assert_eq!(offset_of!(SpaChunk, flags), 12);
    }

    #[test]
    fn pixels_become_opaque_bgra_whatever_order_they_arrive_in() {
        let rgba = vec![10, 20, 30, 0, 1, 2, 3, 4];
        assert_eq!(
            to_bgra(rgba, PixelFormat::Rgba),
            vec![30, 20, 10, 255, 3, 2, 1, 255]
        );
        let bgrx = vec![10, 20, 30, 7];
        assert_eq!(to_bgra(bgrx, PixelFormat::Bgrx), vec![10, 20, 30, 255]);
    }

    /// A real PipeWire video producer (GStreamer's test source) stands in for the desktop, so
    /// this needs a PipeWire server, `pw-cli` and `gst-launch-1.0`; it skips when they are
    /// missing. The portal half is covered in tests/wayland.rs.
    struct Producer(std::process::Child);

    impl Drop for Producer {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn producer(format: &str, name: &str) -> Option<(Producer, u32)> {
        use std::process::{Command, Stdio};
        let child = Command::new("gst-launch-1.0")
            .args([
                "videotestsrc",
                "is-live=true",
                "pattern=solid-color",
                "foreground-color=0xff336699",
                "!",
                &format!("video/x-raw,format={format},width=640,height=360,framerate=30/1"),
                "!",
                "pipewiresink",
                "mode=provide",
                &format!("stream-properties=props,media.class=Video/Source,node.name={name}"),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let producer = Producer(child);
        for _ in 0..50 {
            std::thread::sleep(Duration::from_millis(100));
            let listing = Command::new("pw-cli").args(["ls", "Node"]).output().ok()?;
            let listing = String::from_utf8_lossy(&listing.stdout).into_owned();
            let mut id = None;
            for line in listing.lines() {
                let line = line.trim();
                if let Some(rest) = line.strip_prefix("id ") {
                    id = rest
                        .split(',')
                        .next()
                        .and_then(|number| number.parse().ok());
                } else if line.contains(&format!("\"{name}\"")) {
                    return id.map(|id| (producer, id));
                }
            }
        }
        None
    }

    fn connect() -> Option<OwnedFd> {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
        let socket = std::path::Path::new(&runtime).join("pipewire-0");
        Some(std::os::unix::net::UnixStream::connect(socket).ok()?.into())
    }

    fn centre(picture: &Picture) -> [u8; 4] {
        let at =
            (picture.height as usize / 2 * picture.width as usize + picture.width as usize / 2) * 4;
        let mut pixel = [0; 4];
        if let Some(bytes) = picture.bgra.get(at..at + 4) {
            pixel.copy_from_slice(bytes);
        }
        pixel
    }

    #[test]
    fn pictures_from_a_real_pipewire_node_arrive_as_bgra_in_either_byte_order() {
        if !available() {
            eprintln!("skipped: libpipewire is not installed");
            return;
        }
        for format in ["BGRx", "RGBx"] {
            let Some((_producer, node)) = producer(format, &format!("cc-{format}")) else {
                eprintln!("skipped: no PipeWire server or GStreamer test source");
                return;
            };
            let Some(fd) = connect() else {
                eprintln!("skipped: cannot reach the PipeWire socket");
                return;
            };
            let mut stream = Stream::open(fd, node, Duration::from_secs(10))
                .unwrap_or_else(|error| panic!("{format}: {error:?}"));
            let mut fresh = 0;
            for _ in 0..10 {
                let (picture, new) = stream
                    .next_picture(Duration::from_secs(2))
                    .expect("picture");
                assert_eq!((picture.width, picture.height), (640, 360), "{format}");
                assert_eq!(centre(&picture), [0x99, 0x66, 0x33, 255], "{format}");
                fresh += usize::from(new);
            }
            assert!(
                fresh >= 5,
                "{format}: a live source keeps sending ({fresh} new)"
            );
            let before = Instant::now();
            drop(stream);
            assert!(
                before.elapsed() < Duration::from_secs(5),
                "{format}: stops quickly"
            );
        }
    }

    #[test]
    fn a_vanished_node_is_an_error_not_a_hang() {
        if !available() {
            return;
        }
        let Some((producer, node)) = producer("BGRx", "cc-vanish") else {
            eprintln!("skipped: no PipeWire server or GStreamer test source");
            return;
        };
        let Some(fd) = connect() else {
            return;
        };
        let mut stream = Stream::open(fd, node, Duration::from_secs(10)).expect("opens");
        drop(producer);
        let end = Instant::now() + Duration::from_secs(10);
        let result = loop {
            match stream.next_picture(Duration::from_millis(200)) {
                Err(error) => break Some(error),
                Ok(_) if Instant::now() > end => break None,
                Ok(_) => {}
            }
        };
        assert!(result.is_some(), "the end of the share is reported");
    }
}
