# Notifications

Every message ClipCraft shows to the person (a clip was saved, something failed, a newer version
is out, a name is already taken) follows the one guideline below, so they look and move alike. The
code for it is `apps/clipcraft/src/notify.rs`; build a new message from those parts instead of
drawing your own.

## The four kinds

Pick the kind by what happened, not by how much you want it noticed.

| Kind | Use it for | Badge | Colour | Stays |
|---|---|---|---|---|
| **Success** | something the person asked for is done (saved, copied, renamed, moved) | tick | lime | 8 s |
| **Information** | a note that is neither good nor bad (progress, a change of mode, a draft restored) | "i" | blue | 8 s |
| **Warning** | it worked or carries on, but needs a look (disk getting full, a clip that may be damaged, sound lost) | "!" | amber | 16 s |
| **Error** | it did not work and the person must act or know | cross | red | 30 s |

Worse news is given more time. A message with **Undo** has no time limit: it stays until it is
closed. Time spent with the pointer on a message does not count.

## Three shapes, one look

| Shape | Where | Made with |
|---|---|---|
| **Card** | floats at the bottom of the window, over the page | `notify::card` |
| **Banner** | belongs to a page or panel: the library, the editor, the trim bar, the update notice | `notify::banner`, `banner_actions`, `banner_with` |
| **Inline line** | one line under a field or beside a control (a form error, "Also used by…") | `notify::inline` |

All three use the same parts:

- a round **badge** (28 px): a solid disc in the kind's colour with the kind's glyph in dark ink, so
  the kind is known at a glance (an inline line uses just the glyph, in the kind's colour);
- the message in the normal text colour at 14 px (**never** coloured text in a card or banner;
  only an inline line writes errors and warnings in the kind's colour);
- the large corner radius (14 px);
- buttons on the right, in the standard pill style: at most **Show**, **Undo** and **Delete now**
  on a card, and the page's own few actions on a banner; the primary action is lime;
- a round **close button** (cross) on every card and on every banner the person can ignore.

A card is the panel colour with a soft glow of the kind's colour behind the badge (strongest at the left
edge, gone by the right), a quiet grey outline (the kind is not shown by a coloured border), a deep
soft shadow and, along its bottom edge, a thin track with a line in the kind's colour that shrinks
as its time runs out. The saved-clip pop-up has the same glow, a larger picture and a 20 px title. A
banner has a faint wash of the kind's colour and an outline in the kind's colour instead, and no
banner has a faint wash of the kind's colour and an outline in the kind's colour instead, and no
shadow.
## Motion

One entrance and one exit, the same length everywhere:

- **in:** 0.28 s, ease-out (fast, then settling), rising 16 px while fading in;
- **out:** 0.22 s, ease-in (slow, then quick), sinking while fading out;
- a banner fades in the first time a message appears and does not move;
- the saved-clip pop-up is a window of its own (with rounded corners from the window system on Windows 11, a plain outline elsewhere), so it slides in from the screen's right edge
  (80 px) instead of rising and fading; its timing and easing are the same.

Nothing bounces, flashes or repeats. Reduced work for the computer: a card asks for a new frame
about ten times a second while its time-left line moves, and not at all while it is still.

## Writing the message

- Say what happened first, then what to do. One or two sentences; no codes and no blame.
- Use the person's words: "Saved clip.mp4", not "Operation completed".
- A message about a file offers **Show** (opens the folder with the file selected).
- Do not repeat a message that is already showing; the newest replaces the older one.
- Do not use a card for something the person is looking at already (a toggle that moved shows its
  new state); use an inline line next to the control instead.
- Never use red for anything that is not an error. A warning is amber; a button that deletes is
  red only on its own.

## Where each message lives today

| Message | Shape | Kind |
|---|---|---|
| Clip saved, screenshot saved, renamed, favorited, moved to the Recycle Bin, copy ready, file copied | card | Success |
| Saving, making a copy, refreshing, draft restored, recording mode changes | card | Information |
| Disk getting full, global shortcuts unavailable, replay restarted, recording slowed, a clip that may be damaged, some clips could not be moved | card | Warning |
| Could not save, could not start, disk almost full, ClipCraft went wrong | card | Error |
| Moving clips with **Undo** and **Delete now** | card | Information |
| Saved-clip pop-up (its own window) | card | Success |
| Library errors, editor messages and trim messages | banner | by what it says |
| A newer version is available | banner | Information |
| Form and settings errors (a taken hotkey, a missing key, a bad shape) | inline line | Error or Warning |

## Adding a message

1. Choose the kind from the table above.
2. For a floating message call `capture::notice_as(Kind::…, text, path)` (or `capture::notice`
   for a plain error or information); for a page, `notify::banner`; for a field, `notify::inline`.
3. Do not add a new colour, corner radius, duration or animation. If the guideline cannot say
   what you need, change the guideline and `notify.rs` together and say so in the change.
