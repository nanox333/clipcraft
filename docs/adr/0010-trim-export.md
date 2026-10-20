# ADR 0010: Trim and export without re-encoding

- **Status:** Accepted. Verified with ffprobe and ffmpeg on ClipCraft's own clips and a B-frame file;
  not yet tried on files from other recorders at scale.
- **Date:** 2026-10-20

## Context

Saved clips often need the dead seconds cut off. Re-encoding costs time, quality and CPU while a
game may be running, and clips are already H.264 and AAC. The roadmap's prototype gate asks for a
cut command and a standard video export.

## Decision

`clipcraft-export` (layer 2) copies the compressed pictures and sound of a range into a new MP4:

- **Frame-accurate start without decoding.** Pictures are copied from the last keyframe at or
  before the start, and an MP4 edit list begins playback at the exact start picture. The end is
  the last picture shown before the end time, with an explicit edit duration (B-frame files keep the
  pictures the decoder still needs).
- **Sound.** The AAC unit before the start is kept as the decoder's warm-up; the edit list skips it
  and the part before the cut, so sound and picture stay within a unit (about 21 ms) of each other.
- **Sources.** MP4 and MOV (the source's own edit list and composition offsets are honoured, any
  codec an MP4 can hold is copied) and Matroska/WebM with H.264 and AAC, without B-frames.
- **Safety.** The input is only read, after the media probe's header-size checks; sample counts are
  capped; the output is written to a `.partial` file, renamed only when complete, never over an
  existing file, and removed on error or cancel. Progress is reported and cancel is checked per
  sample.
- **Surfaces.** The engine command `clip.trim` and `trim_clip` (with progress and cancel), the CLI
  `clip trim`, and the clip view's trim row (T, I, O). The new file is `Name (trimmed).mp4` beside
  the original, so the library finds it on its next scan.

## Consequences

- Cheap and lossless, but the cut cannot change resolution, frame rate, or container, and a player
  that ignores edit lists shows up to a keyframe interval of extra picture first. A re-encode of
  only the first group of pictures (smart render) would fix that and is not built.
- Matroska with B-frames and codecs other than H.264/AAC are refused with a reason rather than
  mis-cut.
