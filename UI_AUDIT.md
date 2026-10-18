# UI audit (2026-10-18)

Method: built `dist/Rec.app` (release), launched it, and drove it with real mouse events and screenshots.
Screen Recording and Microphone were **not granted** to Rec, so I could not see the live preview, the
camera scenes, or a recording in progress. Everything below is visible without those permissions.
Not covered: recording, the live preview, scene editing, stream destinations, the menu bar item.

Windows seen: onboarding, main window (1040x660), Settings (520x820, now 520x600).

## Status (second pass, 2026-10-18)

Fixed and checked in the running app (screenshots): 1-4, 6-20, 22, 24, 26-30.

- **6** Settings check boxes are square macOS-style boxes (`widgets::Check`), filled with the accent when on.
- **7** The Microphone check box is labelled "Record".
- **8** The clear buttons use `×`, which the font has.
- **9** Restore Defaults has its own Reset section with an explanation, and asks before acting.
- **10, 11, 29** Settings has tabs: General, Video, Audio, Streaming, Shortcuts, Camera. The camera device and
  overlay are together on the Camera tab. "Locked while recording" sits in the toolbar, so the form no longer
  jumps. The window is now 520x600 instead of 520x820.
- **12** Format, Codec and Folder in the main window show a `›` and open Settings at the matching tab.
- **13** "1080p" in lower case everywhere. Gains show one decimal, and Settings headings match the sidebar's.
- **15** Without microphone permission, Rec records without the mic instead of blocking the preview. The Audio
  card says so and offers Allow… or Open System Settings.
- **16** On the welcome page, the missing required permission gets the primary button.
  "Continue without Screen Recording" is now a plain button. Not seen in the app: all permissions are now
  granted, so the welcome page no longer shows.
- **17** Hover text explains Frame, Tracks, gate, compressor, CBR, keyframes and the live encode, plus a
  note at the top of the Streaming tab.
- **18** The replay switch reads "Keep the last 60 s", with a hint that names Save Replay and its shortcut.
- **19** Without Screen Recording, the record button is dimmed and says why on hover.
- **20** Under Go Live, a hint says what is missing.
- **22** The banner text wraps on its own lines, with the buttons below it. It adds Open System Settings.
- **24** Secondary and tertiary text are lighter in dark mode (darker in light mode).
- **26** Other cameras and displays are in one "Other camera…" and "Other display…" menu each.
  Delete Scene… is on its own row and asks first.
- **27** In a custom scene, the Camera picker is enabled and says "Add a Camera layer below to use the webcam."
- **28** Layer visibility is a check box.
- **30** The compressor's threshold, ratio and makeup gain each have their own labelled row. The noise gate
  does the same.

Not bugs or not changed:

- **5** The meters showed a real signal, then silence.
- **21** The space around the preview comes from keeping the video's shape. Left as is.
- **23** Disabled pickers are now clearly dimmer with the new text colours. No separate change.
- **25** Wrong diagnosis: the crossed-out camera picture was OBS Virtual Camera's own idle image. "Default
  camera" had picked that device, so it wasn't Rec's placeholder.
- **31** The Split template fits the whole screen into half the frame, so the bars are expected. Changing
  it would crop the screen.

New, found and fixed during the retest:

- **32** "Other camera ▾" wrapped one letter per line inside a disabled scope, and ▾ isn't in the font.
  It is now a plain menu button, "Other camera…".
- **33** The delete confirmation ran off the card with a long scene name. The question now sits on its own line.

- **34** In the overlay shape picker, the Squircle icon looked the same as Square. It is now always drawn
  rounded enough to tell apart.
- **35** The scene tile's right-click Delete deleted without asking. It is now "Delete…": it opens the scene
  and shows the same confirmation as Delete Scene….

Nothing found in the retest is still open. The welcome page (16) was not checked in the app.

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
8. **The "clear" buttons show a `□` instead of `✕`.** The bundled font lacks the glyph, so the
   clear-shortcut button (`src/app/ui.rs:1105`), the remove-destination button (`ui.rs:919`) and the
   remove-layer button (`src/app/scenes.rs:365`) all render as an empty square. Rows with no shortcut
   show plain "None" text that does not look editable.
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

## Found with permissions granted (live preview, scenes, recording)

The live preview, the Split scene, the custom-scene editor and a 4 s recording with a pause and resume all
worked, and the file was written to `~/Movies/rec-2026-10-18-120934.mov`.

25. **Camera scenes with no camera access show a crossed-out camera icon and nothing else.** There is no
    text and no button saying the camera is not allowed. Recording starts anyway.
26. **The custom-scene editor lists every camera as an "Add" chip** next to Screen, Image, Video, Web page,
    Text and Color, with no label saying they are cameras. The chips wrap onto five lines. "Export…",
    "Import…" and the destructive "Delete" sit in the same row.
27. **The Camera section stays disabled ("Pick a camera scene to use the webcam.")** while a custom
    scene is open, even though the editor right below it can add a camera.
28. **The layer visibility toggle is a plain white dot** (`●` / `○`), not an eye or a checkbox.
29. **Opening Settings while recording shifts the whole form down** by a line ("Settings are locked while
    recording.").
30. **In the Compressor row the label is centred against three stacked sliders** instead of lining up with
    the first one.
31. **Split scene: the screen is letterboxed** with large purple bars above and below, so the screen
    takes under half of its half.
