# Settings and advanced controls

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
   the process name and available Linux Steam ID. You can also enter the details yourself.
4. Enter the display name and one or more comma-separated process names or Steam app IDs.
   **Match whole process names** selects exact executable matching; when off, process names
   use the existing substring matching. Steam IDs always match exactly.
5. Press **Add custom game**. The saved rule appears in an editable section below the form.
   Use **Remove rule** to remove it. Adding/removing rules changes local preferences, not media.

The form sits directly on the settings page with no card background or enclosing border. It uses
padded inputs, separate labels/help, and a full-width running-app picker. Game and
process names sit side by side when space permits and stack in narrow windows. Existing rules use
the same flat layout, separated by dividers rather than cards or a single crowded row. All
controls have accessibility names and are reachable with Tab; Enter activates the General switch and add/remove actions.

The add button stays disabled until a name and usable process/Steam identifier are provided.
Malformed Steam IDs show an inline error; inputs remain available for correction. Empty or failed
process refreshes show a message and leave manual entry available. Choosing an app without a
Steam ID clears any previously selected app's Steam ID while preserving an explicitly entered
name. The app shows a summary of active custom rules while their advanced editor is hidden.

Limits remain 64 custom rules, up to eight process aliases/Steam IDs per rule and at most 512
unique entries in the running-app picker. Name/process/ID inputs are bounded and the engine's
existing settings validation still applies. Refreshing apps is explicit; merely hiding/revealing
advanced settings does not start a process refresh or recording. Steam process metadata is
Linux-only. Identification still needs a capturable game window and does not establish live-game
or platform compatibility. See [game-catalog.md](game-catalog.md) and [gaps.md](gaps.md).
