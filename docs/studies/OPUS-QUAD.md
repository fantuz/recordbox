# Experimental OPUS-QUAD MIDI profile

Use an OPUS-QUAD to control up to four recordbox decks in PERFORMANCE. The profile is an independently written mapping based on AlphaTheta’s published MIDI addresses. Software replay covers all four decks. Physical response, jog sensitivity, LED appearance and audio routing still need a check on an OPUS-QUAD. [AlphaTheta MIDI message list, E1, pp. 2–3](https://downloads.support.alphatheta.com/software_info/all-in-one-dj-systems/OPUS-QUAD/OPUS-QUAD_MIDI_Message_List_E1.pdf#page=2) · [Profile CSV](../../app/src/midi/profiles/OPUS-QUAD.csv)

## Connect

1. Connect the rear USB Type-C computer port directly to your computer. On the OPUS-QUAD press **SOURCE**, choose **SOFTWARE CONTROL**, then press **LOAD 1/3** or **LOAD 2/4** on each side you want to use. Loading a track from a different source on the unit takes that deck out of control mode. [Operating instructions, pp. 120–121](https://downloads.support.alphatheta.com/manuals/all-in-one-dj-systems/OPUS-QUAD/OPUS-QUAD_DRI1795D_manual.pdf#page=120)
2. In recordbox open **PERFORMANCE → MIDI**, select the primary OPUS-QUAD input and click **Try OPUS-QUAD profile**. Use a four-deck layout for decks 3 and 4. Select the deck layer on the hardware. Clicking **Stop OPUS-QUAD profile** restores the mapping that was active before the trial. Reconnecting loads your usual factory and user assignments.
3. Choose the audio device separately if you want sound through the unit. AlphaTheta specifies a dedicated audio driver for Windows and native audio support on macOS. Linux MIDI replay covers ALSA-style port names. Physical Linux audio support remains unverified. [Operating instructions, pp. 14 and 120](https://downloads.support.alphatheta.com/manuals/all-in-one-dj-systems/OPUS-QUAD/OPUS-QUAD_DRI1795D_manual.pdf#page=14)

## Coverage

| Area | Mapped controls |
| --- | --- |
| Each deck | Play/pause, cue, load, VINYL (jog mode), jog top touch and rotation, rim pitch bend, 14-bit tempo fader, tempo reset/range, master tempo, key sync, beat sync, master, quantize and loop in/out |
| Hot cues | A–H set or call the slot. Shift + A–H deletes that slot through recordbox’s cue editor. The hardware sends separate notes for the shifted buttons. [Operating instructions, p. 81](https://downloads.support.alphatheta.com/manuals/all-in-one-dj-systems/OPUS-QUAD/OPUS-QUAD_DRI1795D_manual.pdf#page=81) |
| Mixer | Trim, three EQ bands, channel fader and headphone cue for each channel. Crossfader, headphones mix/volume and master level are global. |
| Browser | Turn the Smart Rotary Selector to move through the filtered tracks or playlist tree. BACK moves focus from the list to the tree. In the tree, BACK closes an open folder or returns to its parent. Use the deck’s LOAD button to load the selected track. |
| Computer → controller | PLAY blinks once per second while paused or previewing a cue. CUE lights at the cue point and during normal playback or preview, blinks twice per second when paused away from the cue, and goes dark at track end. JOG MODE follows the deck’s Vinyl setting. |

The CSV defines 45 rows. It expands to 162 logical inputs at 166 addresses because each tempo fader reports its coarse and fine values separately. Twelve output addresses carry lamp feedback. You can inspect the CSV offline from a source checkout with `recordbox --midi-coverage app/src/midi/profiles/OPUS-QUAD.csv`.

The trial leaves the center display, jog illumination, hot-cue colors, Beat FX and zone/booth levels to the hardware. Shifted jog rotation and touch, and waveform needle search, are unmapped. The MIDI message list specifies signed jog increments. The physical counts per revolution still need measurement for scratch calibration. Standalone USB library export and OneLibrary use a separate data path.

## Check on a controller

Load a short local track on decks 1 and 3. Keep the mixer output low. Switch the left-hand deck layer and press PLAY once on each layer, watching which software deck starts. Repeat on decks 2 and 4. Pause at the cue point, hold CUE, then press PLAY while holding CUE. Check that the PLAY lamp changes from blinking during preview to steady during playback and that the CUE lamp goes dark at the end of the track. Hot-cue deletion changes the recordbox library, so use a test track to check set, recall and Shift + delete.

Press the **VINYL** button on one deck and check its lamp. AlphaTheta calls this message JOG MODE. In Vinyl mode pressing the top should hold the playing track and turning the top should scratch. In Normal mode the top should pitch-bend. The rim should pitch-bend in either mode. Turn it in both directions during playback and release it. Check that the bend ends. In Vinyl mode, hold the platter top on a playing test track, press SHIFT while holding it, then release the platter. Repeat while changing the deck layer instead of pressing SHIFT. Verify that the original deck resumes and the other deck remains unaffected. If either stays held, record the order of the actions and the MIDI messages from both decks. Sweep one channel fader and one EQ at a time, including channel 4. [Jog wheel modes, pp. 69–70](https://downloads.support.alphatheta.com/manuals/all-in-one-dj-systems/OPUS-QUAD/OPUS-QUAD_DRI1795D_manual.pdf#page=70)

For a hardware report include the firmware version, OS, deck layer, input and output port names, and which inputs and lamps responded. If the trial button is absent, report the exact primary input name. Linux users can list inputs with `aseqdump -l` and observe messages with `aseqdump -p CLIENT:PORT` before opening recordbox. [ALSA MIDI monitor](https://manpages.debian.org/trixie/alsa-utils/aseqdump.1.en.html)
