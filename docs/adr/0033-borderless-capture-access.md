# ADR 0033: Ask Windows for borderless capture access

Accepted 2026-10-28.

Windows Graphics Capture draws a yellow frame around the window or screen being captured. ClipCraft
already sets `GraphicsCaptureSession.IsBorderRequired = false`, but Windows honours that only for an
app that has also been granted borderless access, so the yellow frame could still appear around the
game or desktop at random.

`clipcraft-platform::request_borderless_once` calls
`GraphicsCaptureAccess.RequestAccessAsync(Borderless)` once per run, before the first capture starts,
on a short-lived thread with WinRT initialised (`RoInitialize`, the only `unsafe` call, with a SAFETY
comment). The answer is ignored: where Windows refuses (older versions, or policy) the border stays,
exactly as before. On the development machine the answer is `Allowed` with no prompt.

Other programs that capture (the Xbox Game Bar, Discord, Steam, OBS) draw their own yellow frames;
ClipCraft cannot remove those.
