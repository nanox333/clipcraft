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
the same fade envelope used by export. It uses one player and may pause briefly while opening
the next source. The lower audio lane represents linked source audio; it is not a decoded waveform. Timeline thumbnails are cached source
thumbnails, not frame-by-frame filmstrips. A missing sound device is reported and video preview
continues without sound.

**Export video** opens a window with these output choices:

| Choice | Behavior |
| --- | --- |
| Standard | The existing H.264 MP4 output, up to 720p/30 fps with a 4 Mbps video target |
| Small file | Estimated 10 MB output |
| Medium file | Estimated 25 MB output |
| Custom size | Estimated 1–500 MB output |

The window shows the chosen output dimensions. Size presets reserve the continuous AAC track
and container overhead before estimating the video bitrate; smaller budgets can choose 540p,
480p or 360p instead of 720p. Output is never upscaled. An estimate that cannot fit the audio and
minimum video budget is refused so a larger target can be chosen. Targets are decimal MB and
remain estimates; output can be smaller or larger than the target, and no Discord upload limit
is assumed.

The new MP4 is saved beside the first source. The default name is `Source (edit).mp4`; a changed
title supplies the base name. Progress and **Cancel export** are shown while working. Editing is
disabled during export; navigation and playback remain available. The completion message reports
actual size, the selected target and whether it was exceeded, dimensions and encoder. Exported
media appears on the next library scan if inside the active root. Input files and the in-memory
draft are unchanged. Failures retain the draft and show the reason.

Output is H.264 at 30 fps on a 16:9 canvas whose height follows the first source up to the chosen
resolution cap. Smaller/portrait images fit with black bars without stretching or upscaling.
Video is re-encoded; use quick trim for original-quality sample copy. Video cuts follow the
output frame grid; audio follows the exact timeline and is encoded continuously to avoid per-cut
AAC delay. The output has one 48 kHz stereo AAC track. Only the first source audio stream is
included; mono is duplicated, the first two channels of larger layouts are used, and muted or
silent segments contribute silence. Use quick trim when preserving separate audio tracks matters.
Codec and frame rate stay fixed; GIF and loudness tools remain in quick trim.

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

Navigating away retains edits and imported media; return with the **Editor** rail icon. Closing ClipCraft discards the in-memory draft;
there is no project file or autosave yet. Only an exported MP4 persists the assembled result.

The Rust CLI calls the same engine commands, with at most 16 MiB of JSON on standard input:

```sh
clipcraft-cli editor import < paths.json
clipcraft-cli editor apply < command.json
clipcraft-cli editor export < export-request.json
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
size preset. Omitting `preset`, or setting it to `null`, keeps standard output and remains
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

The `editor.export` JSON result preserves the existing top-level `path`, `duration_ms`,
`pictures`, `audio_tracks` and `bytes` fields. It adds optional `target_bytes`, `within_target`,
`width`, `height`, `encoder` and `hardware_decode`. `target_bytes` is `null` for standard output.
The Rust report stores clip facts in `ExportedSequence.clip`, which is flattened during JSON
serialization; CLI callers do not need to look for a nested `clip` object. The CLI and UI share
the same engine implementation and hardware preference.

No control server, cloud service, .ccproj format or AI upload is involved. Detached audio,
overlapping video tracks, cross-dissolves and general-purpose multitrack editing remain outside
this editor. See [ADR 0019](adr/0019-editor-preview-and-export-presets.md) for the shared rendering
and export decision.
