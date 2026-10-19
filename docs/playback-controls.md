# Playback controls

Opening a clip from the library plays it in the detail view. Every control has a key; the
tooltips name them.

| Control | Key | Notes |
| --- | --- | --- |
| Play / pause | Space, K | Clicking the picture does the same. |
| Seek bar | click, drag | Shows the time under the pointer. Dragging pauses and resumes on release. |
| Back / forward 5 s | ← / → | |
| Back / forward 10 s | J / L | The skip buttons. |
| Previous / next frame | , / . | Pauses first and lands exactly on a picture boundary. |
| Start / end | Home / End | |
| Jump to 0–90 % | 0–9 | |
| Volume | ↑ / ↓ | The slider; 5 % steps. Raising it from zero or mute unmutes. |
| Mute | M | Remembered from clip to clip, with volume and loop. |
| Speed | [ / ] | 0.25×, 0.5×, 0.75×, 1×, 1.25×, 1.5×, 1.75×, 2×, 3×, 4×. Pitch is kept. Starts at 1× for every clip. |
| Loop | R | Restarts from the first picture instead of stopping. |
| Audio track | menu | Shown only when the file has more than one audio stream. |
| Fullscreen | F, double-click | Esc leaves. The controls hide when the pointer rests and return on movement. |

## How it behaves

- Sound is the playback clock. If the sound device stalls without reporting an error, the picture
  carries on after 1.5 s and rejoins the sound when the device delivers again.
- At 1× the sound is passed through untouched. At any other speed it is time-stretched
  (WSOLA) so the pitch does not change.
- Speed, track, and loop changes keep the current position; nothing jumps.

## Not included yet

An output-device picker, a per-clip volume memory, picture-in-picture, subtitles, and chapter
markers. Speed above 4× or below 0.25× is deliberately not offered.
