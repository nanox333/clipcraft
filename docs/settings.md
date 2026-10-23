# Settings and advanced controls

Games also has **Live game data**, separate from capture arming and custom rules. Its CS2/Dota 2/
League switches default off and expose local setup, waiting/error/live summaries and expandable
match details. These regular controls appear before the final advanced section; enabling them
does not restart or change capture. League transport requires Windows/macOS. See
[game integration setup and privacy limits](game-integrations.md).

**Show advanced settings** is the last setting in **Settings › General**, after Library, Startup
and Help. It reveals extra controls at the bottom of the categories below. Each added section
starts with a divider and an **Advanced settings** heading. The navigation categories stay the same.

| Category | Controls revealed at the bottom |
| --- | --- |
| Games | Add a custom game; edit and remove custom rules/overrides |
| Capture | Video aspect ratio: source shape, stretch, black bars or crop |
| Quality | Encoder effort preset and keyframe interval |
| Audio | Sample rate and mono/stereo output channels |

Storage, Hotkeys and About have no extra controls to disclose. They do not acquire empty advanced
sections. Each category keeps its own scroll position. Turning the switch off hides the extra
controls without resetting any values or disabling custom game recognition. The existing local
`show_advanced` preference is reused; it defaults off and survives restarts. Recording changes
continue through the shared `recording.save_settings` command, including the existing restart
notice when an active replay session needs new capture settings.

## Add or override a game

1. Enable **Show advanced settings** at the bottom of General.
2. Open Games and scroll below the searchable built-in catalog to **Advanced settings**.
3. In **Add a custom game**, optionally press **Refresh apps** and choose a running app to fill
   the process name and available Steam ID. You can also enter the details yourself.
4. Enter the display name and one or more comma-separated process names or Steam app IDs.
   **Match whole process names** selects exact executable matching; when off, process names
   use the existing substring matching. Steam IDs always match exactly.
5. Press **Add custom game**. The saved rule appears in an editable section below the form.
   Use **Remove rule** to remove it. Adding/removing rules changes local preferences, not media.

Every settings tab uses one design: a bold heading above a rounded card whose rows are separated by
dividers, in a centred column up to 720 px wide. The custom-game form is a card too, with padded inputs,
separate labels/help and a full-width running-app picker; game and process names sit side by side when
space permits and stack in narrow windows. Existing rules are rows in their own card. Games has a search
box and a switch list. Checked on Linux (Xvfb) only; Windows and macOS are unverified. All
controls have accessibility names and are reachable with Tab; Enter activates the General switch and add/remove actions.

The add button stays disabled until a name and usable process/Steam identifier are provided.
Malformed Steam IDs show an inline error; inputs remain available for correction. Empty or failed
process refreshes show a message and leave manual entry available. Choosing an app without a
Steam ID clears any previously selected app's Steam ID while preserving an explicitly entered
name. The app shows a summary of active custom rules while their advanced editor is hidden.

Limits remain 64 custom rules, up to eight process aliases/Steam IDs per rule and at most 512
unique entries in the running-app picker. Name/process/ID inputs are bounded and the engine's
existing settings validation still applies. Refreshing apps is explicit; merely hiding/revealing
advanced settings does not start a process refresh or recording. Steam IDs can come from Linux
process metadata or verified local installation paths. Identification still needs a capturable game window and does not establish live-game
or platform compatibility. See [game-catalog.md](game-catalog.md) and [gaps.md](gaps.md).

## Automatic recording behavior

Games has a **Switch recording automatically** switch and three choices:

| Mode | When no game is running | When a recognized game opens/closes |
| --- | --- | --- |
| Games only (default) | Idle | Record the verified game window; stop when it exits |
| Games, then desktop briefly | Idle until a game has been captured | Switch to desktop after exit/crash, for the chosen duration; return to a game as soon as detected |
| Desktop and games | Maintain desktop replay, including startup | Switch to each game's window and return to desktop when it closes |

The temporary mode offers 30 seconds, 1/2/5/10 minutes or a custom 15–3,600 seconds. Its default is
one minute. The timer is cancelled when a game opens; closing that game starts a fresh timer. The
top bar shows desktop capture and time remaining. Stop pauses either desktop mode until Start.
Turning automatic switching off returns to manual source selection; it stops any managed session.

These are bounded replay buffers, not unlimited recordings of an entire session. Automatic source
changes and timer expiry save the final replay using the configured replay length and storage
policy. Game clips retain their game name; desktop clips do not inherit it. One transition save
may wait behind one running save; if both are occupied, capture pauses until space is available.
Existing media is never overwritten. Desktop capture may include other apps, so both desktop
modes require an explicit choice. No monitor/permission or a failed game open leaves capture idle
with an error; a Wayland chooser is not opened automatically to work around this.

**Recognize installed Steam games** reads local installation metadata in a background worker,
subject to the main automatic-recognition switch. **Refresh games** refreshes the catalog and
running apps; installed games have per-Steam-ID exclusions. The shared search above also filters
that list. Custom overrides still win. See the catalog guide for root/platform and parsing limits.
