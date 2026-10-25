# Small sequence editor

Open the **Editor** icon on the left rail. It is a separate workspace from **Clips**. Use
**Send to editor** on a clip card or in its detail view to place a reference in the editor library;
this does not immediately add it to the timeline or move the source file. The normal player and
quick trim remain in Clips. The workspace uses the same charcoal surfaces, lime actions,
embedded Inter typefaces and keyboard focus styling as the rest of ClipCraft. Controls are
icon-and-label pills like the rest of the app, with quiet outlined buttons for secondary actions
and lime for the main one. The preview panel is kept minimal: a small title, the picture, a round Play/Pause button in the
centre and the time at the right (scrub on the timeline ruler). Library cards show the file size;
the toolbar keeps edit actions at the left and zoom, Fit and Reset at the right. The timeline has a ruler with minor ticks, a
triangular playhead, grips on the selected segment, and a bar under it with the source in/out,
volume and mute for that segment. A footer lists the keyboard shortcuts. The preview panel is only as wide as a 16:9 picture needs, so the Library takes the rest and
shows as many thumbnail columns as fit (2 to 6); the two panels always have exactly the same height, on every tab. Both can be resized: drag the gap between the Library and
the preview to change their widths, and the gap above the timeline to trade preview height for a
taller timeline (the preview row stops at 210 px). Double-click a gap to go back to the automatic
layout. The sizes last for the session.

When entering the editor directly, **Import clips** in its Library opens a local video picker.
Import can select multiple MP4/MOV/MKV files; files dropped on the editor use the same background
import command. Only references, metadata and cached thumbnails are added. Importing does not
change the normal Clips library or its active folder. The editor media library deduplicates paths
and uses the existing 5,000-media-entry budget; that is independent of timeline segment count.
Import and Send to editor resolve the playable video duration in the background before adding a
source to the timeline. This excludes an AAC padding tail that may extend the container's duration
beyond its video. Older drafts retain their saved out points; if one extends past playable media,
trim that end or remove/re-add the source. Saved edits and originals are never silently shortened.
Automatic export names skip occupied final, partial and recovery paths. An interrupted partial can
remain on disk; a retry chooses a fresh name without deleting it or another instance's live output.

## Arrange and cut

- The left browser contains clips sent or imported into this editor session. Search, right-click
  **Add to timeline**, double-click a thumbnail or drag it onto the timeline. Right-click **Details**
  shows the filename, duration, dimensions, size and path. There is no Add button under each clip. An added clip uses its full source range; trim a long source first
  if adding it would exceed the 30-minute sequence limit.
- Click a segment to select it and move the playhead to its start. Click/drag the ruler to scrub.
  **Play edit**, **Pause edit** and **Start** control the sequence preview.
- **Split (S)** divides the segment under the playhead into two adjacent source ranges. Split at
  least 1/30 second from either edge. No video is changed by editing.
- **Source in / out** edits the selected segment's source start/end, in seconds with millisecond
  input precision. The timeline closes the resulting gap automatically. Bounds must stay within
  the original source, and the retained range must be at least 1/30 second.
- **Duplicate** adds a copy immediately after the selected cut. **Delete** removes that cut and
  closes the gap. It does not delete the source video.
- **Move left/right**, or drag a segment onto another, changes order. There is one gapless track;
  clips cannot overlap. Dragging from the media browser appends to the end.
- **Volume** attenuates the selected segment from 100% to 0%. **Mute** sets 0%; **Unmute** restores
  100%. Audio stays attached to the cut and follows trim, split, duplicate and reorder.
- **Undo/Redo** restores up to 64 edits within an 8 MiB history budget; older history is pruned as needed. **Reset edit** clears the sequence and can be undone.
  **Fit** shows the whole sequence; **−/+** changes horizontal zoom.

Export runs in the background. You can still preview, scrub and use **Space** or **←/→** while it
runs; editing and starting another export stay disabled until it finishes. Progress and cancellation
stay in the top toolbar so they remain reachable in a short window. **Cancel export** changes
to **Cancelling…** while waiting for the current codec operation to return. A cancelled or failed
export leaves the sources unchanged and removes its partial output. Progress updates do not request
more than ten redraws per second, apart from completion; preview and input redraws are independent.
Performance depends on the source and machine; see [measured workloads and limits](performance.md).

The title field names the exported copy; the default uses the first source's filename. A title
is a filename label, not text drawn into the picture. Names are sanitized for local filesystems,
and a numeric suffix is used if a matching output already exists.

| Shortcut | Action |
| --- | --- |
| Space | Play/pause the edit |
| S | Split at the playhead |
| Delete | Remove the selected segment |
| Ctrl/Cmd+D | Duplicate the selected segment |
| Ctrl/Cmd+Z | Undo |
| Ctrl/Cmd+Shift+Z or Ctrl/Cmd+Y | Redo |
| Left/Right | Pause and step the playhead by one 30 fps timeline frame |
| Tab, Enter | Reach and activate buttons/segments |

Shortcuts remain active when a button is focused; text/numeric input retains normal typing.

## Preview and export

Preview plays the original source range and its first audio track at the selected volume, with
the same fade envelope used by export. It prepares the next source paused, then promotes that
source at the cut; its audio output opens only when it becomes active. At most two current,
prepared or retiring players exist, including cancelled decoders until they finish. Slow sources,
seeking or a decoder that is still stopping can still cause a pause; this is not a gapless-playback
guarantee. A missing sound device is reported and video preview continues without sound.

The video lane samples actual pictures from each visible source range. The linked audio lane
shows first-track min/max peaks, with segment volume/mute and fades applied to its display.
Trimming and splitting retain source-time alignment. Background analysis has loading/error states;
it does not block editing or change the source file. These are sampled strips and envelopes, not
every decoded video frame or an editable detached audio track. Media without an audio track has
no waveform; recorded silence produces a flat envelope.

Each analysis accepts 1–12 letterboxed 160×90 picture samples and 16–2048 min/max audio buckets.
Sound comes from the first audio track. MP4/MOV reads native-rate packets and discards decoded
PCM after accumulating peaks. Other containers use an 8 kHz fallback limited to ten seconds and
a nominal native PCM span of 16 MiB; larger requests report unsupported. The requested range is
otherwise at most 30 minutes, and source duration is at most 24 hours. One worker handles
analysis, with up to eight queued UI requests. MP4/MOV metadata is checked before decoding:
at most 16 MiB of movie/fragment metadata, 100,000 boxes/eight nesting levels and one million
declared sample entries across all tracks in the whole source. Declared encoded samples over
64 MiB are also refused before source probing. PCM tables count each audio frame, so
48 kHz PCM longer than about 20.8 seconds can exceed the source budget even for a short
selection. Oversized metadata or audio packets/configuration report unsupported without changing
the source; AAC recording clips use far fewer entries. The 32 MiB/256-entry result cache uploads
at most two textures per poll; source size
and modification time invalidate stale entries. These are result/cache limits; decoder buffers
are separate and can be much larger. Detailed decoder limits and remaining measurements are recorded in
[architecture](architecture.md) and [testing](testing.md).

**Export video** opens a window with these output choices:

| Choice | Behavior |
| --- | --- |
| Standard quality | H.264 MP4 at the selected resolution/FPS, with a 4–50 Mbps video target |
| Small file | Estimated 10 MB output |
| Medium file | Estimated 25 MB output |
| Custom size | Estimated 1–500 MB output |

**Resolution** offers **720p**, **1080p** and **Source · up to 4K**. **Frame rate** offers
**30 FPS** or **60 FPS**. The defaults are 720p/30. Source preserves the first clip's aspect ratio,
including portrait/ultrawide, within a 3840×2160 landscape or 2160×3840 portrait box. The other
two choices retain the existing 16:9 canvas; smaller source pictures are never enlarged.

The window shows actual dimensions, FPS and target video bitrate. File-size targets reserve the
continuous AAC track and mux overhead, then can reduce bitrate and resolution while keeping the
chosen FPS. A visible explanation identifies the reduction. Higher frame rates need more bitrate
for a given resolution. An impossible audio/minimum-video budget is refused. Targets are decimal
MB and remain estimates; output can be smaller or larger than the target, and no Discord upload
limit is assumed. Standard targets scale with pixels/FPS from 4 Mbps to at most 50 Mbps.

The new MP4 is saved beside the first source. The default name is `Source (edit).mp4`; a changed
title supplies the base name. Progress and **Cancel export** are shown while working. Editing is
disabled during export; navigation and playback remain available. The completion message reports
actual size, the selected target and whether it was exceeded, dimensions, FPS and encoder. Exported
media appears on the next library scan if inside the active root. Input files and the in-memory
draft are unchanged. Failures retain the draft and show the reason.

Output is H.264 at the selected 30/60 FPS. The first source determines the canvas as described
above, bounded to 8,294,400 output pixels. Smaller/portrait images fit with black bars without stretching or upscaling.
Video is re-encoded; use quick trim for original-quality sample copy. Video cuts follow the
output frame grid; audio follows the exact timeline and is encoded continuously to avoid per-cut
AAC delay. The output has one 48 kHz stereo AAC track. Only the first source audio stream is
included; mono is duplicated, the first two channels of larger layouts are used, and muted or
silent segments contribute silence. Use quick trim when preserving separate audio tracks matters.
The codec remains H.264; GIF and loudness tools remain in quick trim. Editing/stepping keeps
the existing 30 FPS grid; 60 FPS export samples source motion without interpolation. Shared text
rendering supports these canvases, bounded to 2,097,152 raster pixels per layer and a 16 MiB
active cache; shorten or reduce text if that budget is exceeded. Keyboard focus stays inside the
export window and its popups until it is closed.

Sequence export follows the recording encoder preference. With GPU encoding enabled it tries the
registered hardware encoder, then discards its owned partial output and retries in software if
opening, encoding, stream validation or frame accounting fails. Cancellation stops the job rather
than starting a fallback. Real-GPU speed and quality are unverified; a successful software or
stand-in test does not establish hardware support. Sequence decoding currently stays in
software, so the report's `hardware_decode` field is `false`.

There is no separate segment-count limit. Export remains bounded to 30 minutes total, SDR H.264/AAC MP4/MOV and recorder-compatible MKV. The
existing MKV adapter does not accept reordered video/B-frames. HDR and non-square pixels are
refused. The destination filesystem must support hard links for safe output publication.

## Effects, transitions and text

The left panel has three tabs: **Library**, **Effects** and **Text**. Effects and Text apply to
the item selected on the timeline: a video segment, or a text layer on the **text track** above
the video. A click applies it; clicking the highlighted one again removes it.

**Effects > Transitions.** A segment can fade in from, and out to, black or white over half a
second (or half the segment if it is shorter): *Fade in*, *Fade out*, *Flash in*, *Flash out*. The
presets *Dip to black*, *Dip to white*, *Fade in & out* (first and last segments only) and
*Clear all* set every video segment at once. The picture and the segment's sound fade together, in
the preview and in the export. Cutting a segment keeps only the outer fades. These are dips, not
cross-dissolves: segments never overlap, so the length of the edit does not change. Faded ends
show as triangles on the timeline. A text layer offers *Fade in* and *Fade out*, which ramp its
opacity.

**Effects > Effects.** One colour look per item: Black & white, Sepia, Brighten, Darken,
Contrast, Vivid or Invert, shown on a small colour-bar swatch. On a segment it recolours the
picture (not text or fades); on a text layer it recolours the letters and their shadow. Preview
and export match.

**Text.** Each piece of text is a **layer on its own track** above the video, like a regular
layer. **Add text** in the Text tab puts a 3-second layer at the playhead. On the timeline, drag
its block to move it, drag either end to trim it, click it to select it. With a text layer selected, **Split** (S) cuts it at the playhead into two
layers (each half keeps its own end of the fades), **Duplicate** (Ctrl/⌘ D) puts a copy right
after it, and Delete removes it; with a segment selected they act on the video as before.
Anything it overlaps shows through as a normal layer: it sits above the video, is unaffected by the
video segment's effect and fades, and can be longer or shorter than any one segment. Text layers
are clipped to the length of the edit. Its settings are in the Text tab: the text (one line, up to
120 characters; Enter or clicking away applies it), start and length in seconds, **Size**,
**Colour**, and the *Title*, *Lower third*, *Top banner* and *Corner tag* position presets. Drag
the text in the preview to move it anywhere (one undo step); the position is stored as a fraction
of the picture, so the export puts it where the preview shows it, kept inside the frame. Preview and
export share FilmCraft's Inter SemiBold text raster, layout, shadow, wrapping and placement at
the chosen export canvas size. The preview scales those pixels to its display area. Text rasters
are limited to 2,097,152 pixels per layer, with a canvas-height layout cap and separate 16 MiB
preview/export caches. Moving or fading a layer reuses its pixels.
At most 256 text layers are accepted. Text layers share the video segments' undo history, and
Reset clears both tracks.

The Library shows each clip's resolution and file size under its name.

## Sessions and command line

Navigating away retains edits and imported media; return with the **Editor** rail icon. The
application also keeps one internal local `editor-draft.json` record beside its library index.
It recovers current cuts, text, source references, title, playhead, export preset and quality after a
restart, paused and with a visible notice. Undo/redo history starts fresh. Recovery is metadata;
it does not copy the videos or create a save-project workflow. An ordinary exported MP4 remains
the way to keep or share the assembled result independently of its source files.

Changed metadata is saved after a 500 ms debounce on one serial worker. Normal exit allows up
to two seconds to flush the latest state. A crash, filesystem stall or over-budget edit can leave
the most recent input unsaved; the last successful recovery remains intact. The record is limited
to 16 MiB and at most 5,000 deduplicated absolute media references. Missing sources remain
references, so reconnect the drive or restore the original paths before preview/export.

Damaged, oversized, unreadable or future-version recovery is kept unchanged and shown as an error.
Use the editor's explicit discard action to remove its recovery metadata, or resolve the reported
file-access problem and retry. Discard never deletes source videos or exported copies. The fixed
recovery filename and existing path components cannot be symbolic links. See
[ADR 0020](adr/0020-internal-editor-recovery.md) for atomic-write and failure behavior.

The Rust CLI calls the same engine commands, with at most 16 MiB of JSON on standard input:

```sh
clipcraft-cli editor import < paths.json
clipcraft-cli editor apply < command.json
clipcraft-cli editor export < export-request.json
clipcraft-cli editor timeline < timeline-request.json
clipcraft-cli editor draft-load < recovery-path.json
clipcraft-cli editor draft-save < recovery-save.json
clipcraft-cli editor draft-discard < recovery-path.json
```

`import` accepts a JSON array of local file paths and returns probed media references. It is
read-only and rejects an invalid batch without publishing a partial result. `apply` accepts `{ "session": <previous session>, "action": <typed action> }`. Omit `session` to
start empty. For example (ticks are integers at 254,016,000,000 per second):

```json
{"action":{"action":"append","input":"/clips/example.mp4","name":"Example","duration":2540160000000}}
```

The result is a session that can be supplied to another action, such as
`{"action":"split","at":1270080000000}`. Actions are append, split, trim, move, duplicate,
delete, volume, look, preset, add_text, update_text, delete_text, split_text, duplicate_text, undo,
redo and reset; engine validation is the same as the UI. Durations supplied in an edit payload describe source metadata; export reopens the actual source and validates it.

Export accepts ordered source ranges, optional text layers, an unused output path and an optional
size preset and `quality`. Omitting `quality` retains 720p/30. Omitting `preset`, or setting it
to `null`, uses standard output at the chosen quality and remains
compatible with earlier request payloads:

```json
{"output":"/clips/highlights.mp4","segments":[{"input":"/clips/example.mp4","start":0,"end":1270080000000,"volume_percent":100}]}
```

To use the 10 MB target, add `"preset":{"preset":"small"}`. The 25 MB target is
`{"preset":"medium"}`; a custom 8 MB target is `{"preset":"custom","megabytes":8}`. These are the
values of the request's `preset` field, not separate commands. For example:

```json
{"output":"/clips/highlights-small.mp4","preset":{"preset":"small"},"segments":[{"input":"/clips/example.mp4","start":0,"end":1270080000000,"volume_percent":100}]}
```

Add `"quality":{"resolution":"1080p","fps":60}` for 1080p/60, or use `"source"` for
source framing up to 4K. `"720p"` and 30 are the defaults. Invalid resolutions or rates fail
before source opening/output creation; the same typed settings drive UI, CLI and recovery.

The `editor.export` JSON result preserves the existing top-level `path`, `duration_ms`,
`pictures`, `audio_tracks` and `bytes` fields. It adds optional `target_bytes`, `within_target`,
`width`, `height`, `fps`, target `video_kbps`, `resolution_limited`, `encoder` and `hardware_decode`. `target_bytes` is `null` for standard output.
The Rust report stores clip facts in `ExportedSequence.clip`, which is flattened during JSON
serialization; CLI callers do not need to look for a nested `clip` object. The CLI and UI share
the same engine implementation and hardware preference.

`timeline` accepts a source `path`, integer source `start`/`end` ticks, `frames` (1–12) and
`buckets` (16–2048). It returns exact sample positions, 160×90 RGBA pictures, signed 16-bit min/max
waveform buckets and `has_audio`; this bounded JSON can be large and contains decoded media.
For example:

```json
{"path":"/clips/example.mp4","start":0,"end":1270080000000,"frames":6,"buckets":256}
```

`draft-load` and `draft-discard` accept one JSON string containing an absolute local path ending
in `editor-draft.json`. Load returns the current draft or `null`; discard returns `true` when
successful, including an already-missing file. `draft-save` accepts
`{"path":"/app-data/ClipCraft/editor-draft.json","draft":<draft>}` and returns `true` after
atomic publication. Use the engine's versioned recovery schema, not a hand-authored project file.
The `editor.draft.*` commands enforce the same size, validation, existing-file preservation and
path rules as the UI.

No control server, cloud service, .ccproj format or AI upload is involved. Detached audio,
overlapping video tracks, cross-dissolves and general-purpose multitrack editing remain outside
this editor. See [ADR 0019](adr/0019-editor-preview-and-export-presets.md) for the shared rendering
and export decision.

## Layout notes

The preview takes at least half of the top row and the Library the rest; the timeline fills the window below. Move buttons appear only when there is more than one segment. Trim and volume sit behind **Segment settings** (collapsed by default). **Discard saved draft** is a normal button in the footer.

## Launch and small windows

A restored editor draft no longer takes over the launch screen: ClipCraft opens on Clips, says the
draft was restored, and the Editor tab reopens it. In a short window the editor page scrolls so the
preview keeps a usable height (never under about 250 px) instead of shrinking to a strip.

The Clips toolbar narrows the search box and folder pill together so they never overlap, and the
Storage settings explain why "Delete the oldest clips" is dimmed while no limit is chosen.

## Icons are SVG files

Every interface icon is a plain SVG in `assets/icons/` (24-unit square, `currentColor`, round caps and
joins), so any vector editor can open and change them; each has a licence sidecar and an
`ATTRIBUTION.md` row. `apps/clipcraft/src/svg.rs` is a small built-in renderer for the subset they use
(path with M L H V C Q A Z, circle, rect, line, polyline, polygon; fill and stroke). It adds no
dependency, because a full SVG library (resvg) is MPL-2.0, which `deny.toml` allows only for the audio
decoders. Files over 64 KiB or with unknown content draw nothing instead of failing. This replaces
the earlier icons painted by code.
