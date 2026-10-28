//! Asks Windows to let ClipCraft capture without the yellow border (ADR 0033).
//!
//! Windows Graphics Capture draws a yellow frame around whatever is being captured, unless the app
//! both sets `IsBorderRequired = false` and has been granted borderless access. The capture library
//! sets the first; this makes the second request, once per run, and ignores the answer: where
//! Windows refuses (older versions, or policy) the border simply stays.
#![cfg_attr(windows, allow(unsafe_code))]

/// Requests borderless capture access the first time it is called; later calls do nothing.
#[cfg(windows)]
pub fn request_borderless_once() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let worker = std::thread::Builder::new()
            .name("borderless-access".into())
            .spawn(|| {
                use windows::Graphics::Capture::{
                    GraphicsCaptureAccess, GraphicsCaptureAccessKind,
                };
                use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
                // SAFETY: Initializes WinRT for this short-lived thread only; a failure (already
                // initialized differently) is harmless and ignored.
                let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
                if let Ok(request) =
                    GraphicsCaptureAccess::RequestAccessAsync(GraphicsCaptureAccessKind::Borderless)
                {
                    let _ = request.join();
                }
            });
        // The first capture waits for the answer, which comes back at once.
        if let Ok(worker) = worker {
            let _ = worker.join();
        }
    });
}

/// There is no border to remove on other systems.
#[cfg(not(windows))]
pub fn request_borderless_once() {}

#[cfg(all(test, windows))]
mod tests {
    /// Shows what this Windows answers; run by hand: `cargo test -p clipcraft-platform borderless -- --ignored --nocapture`.
    #[test]
    #[ignore = "asks the operating system"]
    fn windows_answers_the_borderless_request() {
        use windows::Graphics::Capture::{GraphicsCaptureAccess, GraphicsCaptureAccessKind};
        use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
        // SAFETY: Initializes WinRT for the test thread.
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
        let status =
            GraphicsCaptureAccess::RequestAccessAsync(GraphicsCaptureAccessKind::Borderless)
                .and_then(|request| request.join());
        println!("borderless access answer: {status:?}");
        super::request_borderless_once();
    }
}
