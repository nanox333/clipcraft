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
shows as many thumbnail columns as fit (2 to 6); the two panels keep the same height. Both can be resized: drag the gap between the Library and
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

Preview plays the original source range and its first audio track at the selected volume. It uses
one player and may pause briefly while opening the next source. The lower audio lane represents
linked source audio; it is not a decoded waveform. Timeline thumbnails are cached source
thumbnails, not frame-by-frame filmstrips. A missing sound device is reported and video preview
continues without sound.

**Export video** renders the ordered cuts into a new MP4 beside the first source. The default name
is `Source (edit).mp4`; a changed title supplies the base name. Progress and **Cancel export** are
shown while working. Editing is disabled during export; navigation and playback remain available.
Exported media appears on the next library scan if inside the active root. Input files and the
in-memory draft are unchanged. Failures retain the draft and show the reason.

Current output is H.264 at 30 fps, with a 16:9 canvas whose height follows the first source up to
720p. Smaller/portrait images fit with black bars without being stretched or upscaled. Source
video is re-encoded, so this path is not original-quality sample copy. Video cut timing is sampled
on the output frame grid; audio follows the exact timeline and is encoded continuously to avoid
per-cut AAC delay. The output has one 48 kHz stereo AAC track. Only the first source audio stream
is included; mono is duplicated, the first two channels of larger layouts are used, and muted or
silent segments contribute silence. Use quick trim when preserving separate audio tracks matters.
There is no sequence-export size preset or adjustable codec/FPS in this slice.

There is no separate segment-count limit. Export remains bounded to 30 minutes total, SDR H.264/AAC MP4/MOV and recorder-compatible MKV. The
existing MKV adapter does not accept reordered video/B-frames. HDR and non-square pixels are
refused. The destination filesystem must support hard links for safe output publication.

## Effects and text

The left panel has three tabs: **Library**, **Effects** and **Text**. Effects and Text apply to
the segment selected on the timeline; a click applies it and clicking the highlighted one again
removes it.

**Effects > Transitions.** A segment can fade in from, and out to, black or white over half a
second (or half the segment if it is shorter): *Fade in*, *Fade out*, *Flash in*, *Flash out*. The
presets *Dip to black*, *Dip to white*, *Fade in & out* (first and last segments only) and
*Clear all* set every segment at once. The picture and the segment's sound fade together, in the
preview and in the export. Cutting a segment keeps only the outer fades. These are dips, not
cross-dissolves: segments never overlap, so the length of the edit does not change. Faded ends
show as triangles on the timeline.

**Effects > Effects.** One colour look per segment: Black & white, Sepia, Brighten, Darken,
Contrast, Vivid or Invert, shown on a small colour-bar swatch. It is applied to the picture
(not to text or fades) in the preview and in the export.

**Text.** One line (up to 120 characters) shown over the whole segment, in white bold Inter with
a soft shadow. Type it, press Enter or click away to apply, and choose *Title* (middle), *Lower
third* (bottom) or *Top banner*; *No text* removes it. To show it for only part of the time, cut
the segment first. The export lays text out with FilmCraft's text engine; the preview draws it
approximately the same. Segments with text show a **T** badge.

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
delete, volume, undo, redo and reset; engine validation is the same as the UI. Durations supplied
in an edit payload describe source metadata; export reopens the actual source and validates it.

Export accepts an ordered list of source ranges and an unused output path:

```json
{"output":"/clips/highlights.mp4","segments":[{"input":"/clips/example.mp4","start":0,"end":1270080000000,"volume_percent":100}]}
```

No control server, cloud service, .ccproj format or AI upload is involved. Text, effects,
transitions, detached audio and multitrack editing remain outside this first slice.
