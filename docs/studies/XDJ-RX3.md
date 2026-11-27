# Experimental XDJ-RX3 MIDI profile

Use the XDJ-RX3 as a two-deck MIDI controller for recordbox in **PERFORMANCE**. The profile uses messages published by AlphaTheta. Verification so far covers software replay. Physical response, feedback LEDs and audio routing still need a check on an RX3. [XDJ-RX3 MIDI message list, E1](https://downloads.support.alphatheta.com/software_info/all-in-one-dj-systems/XDJ-RX3/XDJ-RX3_MIDI_Message_List_E1.pdf)

## Connect the two decks

1. Connect the computer to the rear USB-B port on the RX3.
2. Press **SOURCE**, choose **SOFTWARE CONTROL**, then press **LOAD 1** and **LOAD 2** to assign both decks. You can also assign just one. Leave the deck selectors on 1 and 2 for this profile.
3. Open recordbox in **PERFORMANCE**, open the **MIDI** window and select the RX3's main MIDI input.
4. Click **Try RX3 profile**. Click **Stop RX3 profile** to return to the mapping you were using. The trial applies to this connection and leaves saved assignments on disk.

Loading a track from another source on the RX3 takes that deck out of software control. The mixer can continue to operate as a standalone mixer. To use mapped software mixer controls, choose **Utility** or **Shortcut → MIXER MODE → MIDI CONTROL** on the unit. Audio-device selection is separate. AlphaTheta documents its dedicated audio driver for PC and Mac. [Operating instructions, pp. 110–111 and 116](https://downloads.support.alphatheta.com/manuals/all-in-one-dj-systems/XDJ-RX3/XDJ-RX3_DRI1702C_manual.pdf#page=110)

## Controls in the trial

| Group | Controls |
| --- | --- |
| Decks 1 and 2 | PLAY/PAUSE, CUE, jog top touch and rotation, jog rim pitch bend and 14-bit tempo slider |
| Pads in HOT CUE mode | Eight unshifted pads per deck, input only |
| Mixer in MIDI CONTROL mode | Trim, high/mid/low EQ and channel fader on each channel, plus the crossfader |
| Feedback from recordbox | PLAY and CUE lamps on each deck |

The CSV has 20 input rows. Deck expansion gives 39 logical bindings on 41 input addresses because each tempo slider uses two CC addresses. Two output rows drive the four lamps. You can inspect the profile offline with `recordbox --midi-coverage app/src/midi/profiles/XDJ-RX3.csv` from a source checkout. [Profile CSV](../../app/src/midi/profiles/XDJ-RX3.csv)

## Check the hardware

Load a short local track on each software deck. Press PLAY on physical deck 1 and then deck 2. Check that each starts its own track. Next click PLAY in recordbox to check that the correct physical lamp follows a change originating on the computer. Repeat with CUE. For a test report, record the firmware version from **Utility → GENERAL → VERSION No.**, the OS, the exact MIDI port names and the controls that responded in each direction. A Linux hardware report gives the example port name `XDJ-RX3:XDJ-RX3 MIDI 1 28:0`. The ALSA address can vary. [Linux port report](https://github.com/chrisle/rbxport/pull/274)

Screen browsing, waveform transfer, jog displays, calibrated level-meter output and standalone USB export are outside this MIDI profile. The RX3 publishes its own display encodings. Its 14-bit jog display uses two CC addresses spaced by 0x20, and its channel meter has an explicit dB-to-CC table. The five-segment WeGO meter in recordbox uses a different encoding. [RX3 display and meter tables](https://downloads.support.alphatheta.com/software_info/all-in-one-dj-systems/XDJ-RX3/XDJ-RX3_MIDI_Message_List_E1.pdf)
