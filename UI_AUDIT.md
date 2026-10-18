# UI audit (2026-10-18)

Method: built `dist/Rec.app` (release), launched it, and drove it with real mouse events and screenshots.
Screen Recording and Microphone were **not granted** to Rec, so I could not see the live preview, the
camera scenes, or a recording in progress. Everything below is visible without those permissions.
Not covered: recording, the live preview, scene editing, stream destinations, the menu bar item.

Windows seen: onboarding, main window (1040x660), Settings (520x820).

## Status

Items 1-4 and 14 are fixed (verified in screenshots). Item 5 was not a bug: the meters showed a
live signal (-73 dB) for a moment, then silence (-∞) once nothing was playing. The rest are open.

## Bugs (broken or clipped)

1. **Settings window clips its content on the right.** The Settings window is 520 pt wide
   (`src/app/mod.rs:643`). Sliders and fields run past the edge. "3.0" next to Compressor is cut
   off, and so are the Audio bitrate and Camera dropdowns. The Camera Overlay row of shape buttons
   is cut off (the fourth button is half visible). Shadow and Mirror show only a label, with no visible control.
2. **Main window shows the onboarding page and the full scene panel at once.** The right-hand
   panel stays live and clickable while "Welcome to Rec" is up. A user can change scene or output
   settings before granting any permission.
3. **Scene labels are truncated:** "Screen + Came", "Camera + Scre". The "Frame" dropdown
   shows "Source sha...". The Screen source dropdown is empty with no placeholder.
4. **The screen source dropdown is blank**, and the second row ("Entire display") is clipped at the
   bottom of the panel. There is no sign that the list is empty because of the missing permission.
5. **Meters changed from `-73` to `-∞`** after onboarding was dismissed, with no input change. Check
   that this is not a flicker or a stale reading.

## Standards and platform conventions

6. **Checkboxes are round**, like radio buttons (HDR, Bitrate "Auto", Cursor, Mic filters, Record).
   macOS uses square checkboxes, and round ones read as "pick one". Mic filters is four
   independent options that look like a radio group.
7. **The Microphone row has a checkbox with no label** beside the device dropdown. It is unclear what
   it toggles.
8. **Shortcut recorder buttons show a bare `□` glyph.** It looks like a missing icon, not a
   "record shortcut" control. Rows with no shortcut show plain "None" text that does not look editable.
   There is no way to clear a shortcut.
9. **"Restore Defaults" sits under the Help section** with no confirmation visible, and it is
   not tied to any section. A destructive action should be separated and confirmed.
10. **"Camera Overlay" appears after Help and Restore Defaults**, at the bottom, as a separate
    heading. It belongs next to Camera. Settings has two headings for camera.
11. **Settings opens as a window titled "Settings" with no tab bar.** The macOS convention is
    toolbar tabs or a sidebar. About 40 controls are in one 820 pt scroll.
12. **Main window rows look like buttons but don't say so.** Format "1080P · 60 fps", Codec
    "Auto · .mov", Folder "~/Movies" are right-aligned plain text with no chevron or border.
    It is unclear that they are clickable, or that they differ from the real Settings controls.
13. **Capitalisation and unit styles are inconsistent:** "1080P" in the main window vs "1080p" in
    Settings, "60 fps", "192 kb/s", "-45 dBFS" next to "-20.0 dB", "0 dB" next to "0.0 dB",
    "2.00 s" vs "60 s". Section headings use a mix of ALL CAPS (main) and small caps (Settings).

## Confusing wording

14. **The preview error shows a CLI command to a GUI user:** "run \`rec permissions --request\`,
    then enable Rec in System Settings > Privacy & Security". Source: `src/record.rs:158`. The banner
    below it already has a "Request Permissions" button, so the instruction conflicts with it.
15. **Microphone is "optional" on onboarding but is listed as a missing permission** in the banner,
    and it is on by default. A user who skipped it is told the preview is unavailable because of it.
16. **"Continue without Screen Recording" is the blue primary button.** It is the path that
    leaves the app unusable. The primary should be the one that fixes the problem (Allow, or Open
    System Settings). "Restart Rec" has no explanation of when it is needed.
17. **Jargon without explanation:** "Studio mode", "Frame: Source shape", "Gate opens at",
    "threshold", "Keyframe every", "Tracks: Mix + separate system & mic", "Live encode
    (replay & streaming)". A short hint under each, or hover text, would help.
18. **"Keep recent footage" in the main window vs "Replay buffer" as its heading, and "Save replay"
    in Shortcuts.** Three names for one feature. The same applies to "Go Live" vs "stream".
19. **The record button has no label.** It is a bare red circle, and it stays active even though
    the preview is unavailable and the required permission is missing.
20. **Stream key field has a "Save" button but no visible state** (saved or not), and "Go Live" is disabled
    with no reason shown.

## Layout and polish

21. **Dead space.** The main window has a large empty "No preview" box with a red banner above
    it. The banner pushes the preview down by about 70 pt when it shows.
22. **The banner's buttons ("Request Permissions", "Dismiss") are small and crowded** against the
    text, and the banner text is full-width, so it wraps awkwardly.
23. **The camera hint "Pick a camera scene to use the webcam." sits under a disabled dropdown.** The
    dropdown's disabled state is not clearly different from enabled (low contrast).
24. **Contrast:** grey secondary text (descriptions, "Not asked yet" pills, units) is low contrast on
    the dark background, and the disabled sliders in Settings are almost invisible.

## Not done

- No fixes were made. The only change in the working tree besides this file is your own
  uncommitted edit to `src/app/onboarding.rs`, which was included in the build I tested.
- Re-run the audit after granting Rec Screen Recording and Microphone, to check the preview,
  scenes, recording flow and the menu bar item.
