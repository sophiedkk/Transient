# Manual tests

What the automated checks cannot judge: how things *sound*, and how they feel
by hand. The owner runs these in the installed development Flatpak before a
release; each section says when it was last confirmed. Report problems with
the section and step number (for example "B3").

Start "Transient (Development)" from the app grid, or
`flatpak run io.github.sophiedkk.Transient.Devel`. Use headphones or speakers
at a normal volume. Test files are in `test-audio/` (`dev/fetch-test-audio.sh`).
For the noise gate and compressor, a recording of your own voice with pauses
is the best material: the test collection's speech has too much background
noise for a gate.

## Milestone 15: Effects

### A. Low Cut

*Confirmed 2026-11-06 ("It works!"), before the 20–500 Hz range and Slope
were final; recheck steps 4–6.*

1. Drag `drum-loop.wav` into the window. Press Space to play.
2. Open the track's menu (⋮) and choose **Effects**.
   - The right pane opens on its **Track** tab ("Clip" and "Track" tabs at the
     top), with the track name and a **Low Cut** group: a switch (off), a
     **Cut Below** slider showing its frequency, and a **Slope** row. The
     slider and Slope are greyed out while the switch is off.
3. While it plays, turn **Low Cut** on.
   - The kick loses a little of its lowest boom. No click when switching.
4. Drag **Cut Below** slowly all the way to **Thin**.
   - Clearly audible, also on laptop speakers: the kick ends up a "tick". The
     value beside the title follows, up to 500 Hz. No crackle while dragging.
5. Drag it back towards **Rumble only** until it shows about 40 Hz.
   - The boom comes back.
6. At about 200 Hz, switch **Slope** between Gentle, Medium and Steep.
   - Gentle keeps more of the low end, Steep cuts hardest. No click.
7. Turn Low Cut off: the full sound returns without a click; the slider and
   Slope grey out.

### B. Noise Gate

*Tried 2026-11-06 without a clear result: the test files are not suited (see
below). Retest with the recipe.*

A gate only acts in the pauses, and only when the noise there is clearly
quieter than the voice. Music has no pauses, and the test collection's speech
has noise just 13 dB under the voice, which no gate can separate. Make a
recording that shows it:

1. Somewhere with a steady, quiet noise (a fan, a laptop, a fridge), record
   yourself with Transient for about 20 seconds: a sentence, a pause of two
   or three seconds, another sentence, another pause. Speak at a normal
   distance from the microphone.
2. Play it with the gate off and listen to the pauses: you hear the fan or
   hiss there. Watch the level meter in a pause and while you speak.
3. Turn **Noise Gate** on and set **Reduction** to **Silence** (the clearest
   to hear). Drag **Quiet Below** to the right while it plays, until the
   pauses go completely silent but no word is cut.
   - Too far left: the pauses keep their noise (the gate never closes).
   - Too far right: quiet words or word endings drop out too.
   - Right: dead silence between sentences, words start crisply and their
     ends fade out over a moment instead of being chopped.
4. Switch **Reduction** to **Strong** and then **Light**: the noise comes back
   in the pauses, quieter (−20 dB, −10 dB). Light usually sounds most
   natural; Silence can sound like the line went dead.
5. No clicks when the gate opens or closes, or when dragging.

### C. Equalizer

*Confirmed 2026-11-06 with music ("the effects I would expect").*

1. On a music or voice track, switch **Equalizer** on (between Noise Gate and
   Compressor). Drag **Bass** to the right: more boom and body; to the left:
   thinner. The value shows "+3 dB", "0 dB", "−3 dB" in whole decibels.
2. **Middle** to the right: more presence (a voice comes forward, can sound
   nasal at the top); to the left: more hollow.
3. **Treble** to the right: brighter, more air (and more hiss); to the left:
   duller, softer "s" sounds.
4. All three at 0 dB: the sound is exactly as with the Equalizer off.

### D. Compressor

*Not yet tested by hand.*

1. On a voice track with loud and quiet words, scroll down in the Track tab
   and switch **Compressor** on (Compress Above −20 dB, Ratio Medium 4:1).
   - Quiet words come up, loud words stay about as loud: the voice sounds
     more even and closer. No pumping or clicks.
2. Drag **Compress Above** towards **Almost all** and try the **Ratio**
   choices.
   - Lower and stronger = flatter, more "radio". Strong at a very low
     threshold may sound squashed; that is expected.
3. With Low Cut, Noise Gate and Compressor all on: the pauses stay quiet (the
   compressor runs after the gate, so it does not bring the noise back up).

### E. Prevent Clipping (master limiter)

*Confirmed 2026-11-06.*

1. Right of the master volume (bottom bar) is a toggle with a "flat top" icon,
   blue while on (tooltip "Prevent Clipping"). It is on for new projects and
   for projects saved before.
2. Import three or more loud files (drum-loop, piano, clapping) and raise the
   master volume to the top.
   - On: louder but clean; the level meter never turns red.
   - Off: it distorts and the meter turns red. Ctrl+Z turns it back on.

### F. Track headers and the effects button

*Confirmed 2026-11-06.*

1. Each header shows the name on its own line, with mute, solo, the effects
   button (faders icon) and ⋮ below it. Headers line up exactly with their
   lanes, also with many tracks (scroll down: no growing offset).
2. The effects button is grey (tooltip "Effects") until any effect of the
   track (low cut, gate, equalizer, compressor) is on, then blue ("Effects On"). With two effects on, it stays blue
   until both are off. Clicking it opens that track's Track tab.
3. Mute turns the speaker orange; solo turns the headphones blue. Check in the
   dark style too.
4. Rename a track (double-click its name, or F2): the text field fits in the
   name's line, and the header does not change height.

### G. The pane: Clip and Track tabs

*Confirmed 2026-11-06.*
1. Open the pane (top right button, or F9) and click a clip: the Clip tab
   shows its Gain and Fades; the Track tab shows the effects of its track.
   Clicking a clip on another track makes the Track tab follow.
2. Choose Effects in another track's menu: the pane switches to the Track tab
   for that track and the clip is deselected. Double-clicking a clip switches
   back to Clip.
3. Two tracks: each has its own settings (switch one on, check the other).
4. Open a track's effects, then remove the track (⋮ → Remove Track): the Track
   tab shows "No Track Selected"; Undo in the toast brings it back.
5. F9 closes and opens the pane; the button's tooltip is "Properties", and the
   Keyboard Shortcuts (Ctrl+?) say "Show Properties".

### H. Undo, saving, opening, exporting

*Confirmed 2026-11-06.*
1. Drag Cut Below once, then Ctrl+Z: it goes back in one step (not one per
   tiny move). Ctrl+Z again undoes switching it on; Shift+Ctrl+Z redoes.
2. Move any slider or toggle mute in a saved project: the title gets its "•"
   right away.
3. Set effects on a track, save (Ctrl+S), close, open again: same settings,
   same sound.
4. Export to WAV and play it in another player: the effects are in it.
5. Open a project saved with 0.3.0 (or `test-audio/test-project.transient`):
   it opens, all track effects off, Prevent Clipping on, sounds as before
   (unless it clipped). Saved again, it is format 3, which 0.3.0 refuses.

### I. No clicks when changing things during playback

*Confirmed 2026-11-06.*
Quiet, steady sound shows clicks best (`speech-mono.wav`, `piano.flac`).

1. While playing, type `-12` in the clip's **Gain** and press Enter, then `0`:
   the level changes at once but smoothly, without a click.
2. Click Gain's − and + quickly several times: no clicks.
3. Drag the track's volume and pan, and the master volume, quickly: no
   crackle.
4. Toggle mute, solo and Prevent Clipping while playing: clean, no pop.
5. "Fade In to Playhead" while playing: no click.

### J. Seeking with effects on

*Confirmed 2026-11-06.*
1. With Cut Below near Thin (and the compressor on), click around the
   timeline while playing, and move the playhead with the arrow keys: each
   jump starts cleanly, without a thump.
2. Export a time selection from the middle of a clip: the file starts
   cleanly.

### K. Keyboard and looks

*Confirmed 2026-11-06.*
1. Tab into the pane: Space toggles a switch, the arrow keys move a slider,
   Enter opens a choice row.
2. Make the window narrow: the pane slides over the timeline and still fits.
3. Dark style: the pane and its captions are readable.
