//! Help > Online Manual: the user guide, written as one HTML page into the library folder
//! (so it always matches this version and the current keyboard shortcuts) and opened in the
//! browser.

use crate::app::Prefs;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// (anchor, title, body as HTML).
const CHAPTERS: [(&str, &str, &str); 14] = [
    ("start", "Getting started", r#"
<p>recordbox is a local DJ library: it analyses your tracks, lets you prepare cues, loops, beat grids and playlists, and exports USB drives that Pioneer DJ / AlphaTheta players (CDJ, XDJ) read.</p>
<ul>
<li><b>Your music files are read-only during desktop sync.</b> Editing a track saves your work in Recordbox's library.</li>
<li>The collection footer shows your active recordbox library. Use the lower-left arrows to <a href='#desktop-library-sync'>choose the direction of desktop library sync</a>.</li>
<li>Choose <i>File &gt; Show Library Folder</i> to open the active recordbox data folder in your file manager.</li>
</ul>
<h3>The window</h3>
<p>Top: the player (waveforms, transport, cues, grid). Below: the browser with the Tree View on the left (Collection, Playlists, Related Tracks, Explorer, Devices, Histories) and the track list on the right. <i>View</i> switches between <b>1 Player</b>, <b>2 Players</b>, <b>Simple Player</b> and <b>Full Browser</b>.</p>
<p>In <b>EXPORT</b> and <b>PERFORMANCE</b>, click the overlapping windows at the far right of the top bar to open the library browser in its own window. The decks, GRID and MEMORY controls stay in the main window. Everything below them moves together, including the collection tree, playlists, filters, search and track list. You keep your selection and place in the list. Close the library window or click the same icon again to continue browsing in the main window.</p>
<p>You can move, resize or maximize each window independently. To fill two monitors, move the library to the second monitor, then press <kbd>F11</kbd> in either window. Both windows enter fullscreen on their own monitors. On one monitor, the window where you pressed <kbd>F11</kbd> stays in front so you can keep working there. Press <kbd>F11</kbd> again to restore both windows. With the library attached, <kbd>F11</kbd> toggles the main window.</p>
<p><kbd>Space</kbd> plays or pauses the active deck in both modes. When editing a search or text field, it types a space. To switch windows, use <kbd>Alt+Tab</kbd> on Windows or most Linux desktops. On macOS, use <kbd>Command+`</kbd> for windows in the same Space, or Mission Control for windows in different fullscreen Spaces.</p>
"#),
    ("import", "Importing music", r#"
<ul>
<li><i>File &gt; Import &gt; Import Track</i> adds files; <i>Import Folder</i> adds a folder and everything inside it. You can also drag files or folders from Finder onto the track list or a playlist.</li>
<li><i>File &gt; Import &gt; Import Playlist</i> reads an M3U/M3U8 playlist and creates a playlist with its tracks.</li>
<li><i>File &gt; Import Collection from rekordbox xml…</i> reads a collection exported from rekordbox (tracks, cues, grids, playlists).</li>
<li><i>File &gt; Review desktop library sync…</i> opens the direction chooser for the installed AlphaTheta library and the active recordbox library.</li>
<li>The <b>Explorer</b> source browses folders on disk; tracks there can be loaded or imported.</li>
</ul>
<p>Supported files: MP3, AAC/M4A, WAV, AIFF, FLAC, ALAC. With <b>Auto Analysis</b> on (Track &gt; Auto Analysis, or Preferences &gt; Analysis) new tracks are analysed as they arrive.</p>
<h3 id="desktop-library-sync">Desktop library sync</h3>
<p>Open the arrows at the bottom left and check the two paths. <code>master.db</code> belongs to AlphaTheta Rekordbox. <code>library.db</code> is your active recordbox collection.</p>
<p><b>Import · AlphaTheta Rekordbox → recordbox</b> imports from a private snapshot of the desktop database and its write-ahead log. The import can read analysis files, tags and artwork for local audio. Confirming imports metadata, cues, grids and My Tags into recordbox. The imported <b>rekordbox</b> playlist folder is rebuilt each time. Keep playlists you edit outside that folder.</p>
<p><b>Write back · recordbox → AlphaTheta Rekordbox</b> opens the write-back pilot. Preview the changes, check the destination, select eligible edits and review them before applying. Quit AlphaTheta Rekordbox and its background agent before writing. The confirmed operation saves a verified recovery copy and applies the selected changes to the original desktop library. Start with a disposable AlphaTheta profile for the version you use.</p>
<p>For new cue points, cue timing edits and more complex tempo maps, use <i>File &gt; Export Collection in xml format</i> and review a manual XML import in a disposable AlphaTheta profile.</p>
<p><i>File &gt; Library &gt; Backup Library</i> saves your recordbox collection. To import from AlphaTheta Rekordbox at launch, enable <b>Import the installed AlphaTheta Rekordbox library at startup</b> in <i>Preferences &gt; Advanced &gt; Others</i>. Writing back always requires a manual review. Device Sync Manager can write playlists and media to a selected USB drive.</p>
"#),
    ("analysis", "Track analysis", r#"
<p>Analysis finds the BPM and beat grid, the key, the waveforms, the loudness (for Auto Gain), phrases and vocal positions.</p>
<ul>
<li><i>Track &gt; Analyze Track</i> analyses the selected tracks again (their grid is replaced unless <b>Analysis Lock</b> is on).</li>
<li><i>Track &gt; Analyze Missing Data</i> only fills in what a track does not have yet.</li>
<li><i>Track &gt; Analyze Key</i> finds the key again.</li>
<li><i>Track &gt; Analysis Lock</i> protects a track's grid and key from re-analysis and from grid edits.</li>
<li><i>Track &gt; Stop Analyzing</i> empties the queue.</li>
</ul>
<p>Preferences &gt; Analysis sets the <b>Analysis Mode</b> (Normal for steady tempo, Dynamic for tempo that drifts), the <b>BPM Range</b>, and which items are analysed. <b>CUE Analysis</b> can place a memory cue on the first beat and hot cue A ("1.1Bars") on the first downbeat when a track is first analysed.</p>
<p>To check where an imported MP3 cue lands against a drum hit, open <b>Preferences &gt; Analysis &gt; Track Analysis &gt; MP3 cue timing</b>. It shows which timing is active and how to audition the same cue in a separate collection. Starting recordbox normally returns to your usual collection.</p>
"#),
    ("browser", "The browser and track list", r#"
<ul>
<li>Click a column header to sort; right-click it to choose columns; drag the edges to resize. Playlists and the Collection keep separate column sets.</li>
<li>The search field filters the list (the magnifier menu chooses which field is searched). <i>View &gt; Filter</i> opens the Track Filter: BPM range, key, rating, colour and My Tags.</li>
<li>The <b>column browser</b> (Genre / Artist / Album) and the <b>Sub-Browser Window</b> (a second list under the main one) are switched in <i>View</i> and Preferences &gt; View &gt; recordbox.</li>
<li>Right-click a track for its menu: load, analyse, add to playlist, colour, rating, My Tag, Property, Show in Finder, Remove from Collection.</li>
<li><i>Track &gt; Property</i> edits a track's information. <i>Track &gt; Reload Tag</i> reads the tags from the file again.</li>
<li>Removed tracks can be brought back for 30 days: <i>Track &gt; Restore Removed Tracks…</i></li>
<li>The <b>Preview</b> column plays from where you click; it shows hot cue markers when Preferences &gt; View &gt; Layout &gt; Display Cue Markers on Preview is on.</li>
</ul>
<h3>Missing files</h3>
<p>A track whose file has moved shows a warning mark. <i>File &gt; Display All Missing Files</i> lists them. <i>Track &gt; Relocate &gt; Relocate</i> points one track at its file; <i>Auto Relocate</i> searches the folders listed in Preferences &gt; Advanced &gt; Browse; <i>File &gt; Relocate Missing Files…</i> searches a folder you choose.</p>
<p>Both searches use the saved filename, including names from Windows drives and shares. When a track has a recorded size, one candidate must match that size. With an unknown size, a single available file with the same name can match. The summary distinguishes size conflicts, multiple candidates, unverified files and paths already used by another track. If two missing tracks both match one file, both stay in the list for individual review. Use Relocate to choose a file yourself. Relocation updates the path in your Recordbox collection.</p>
<h3>Review collection cleanup</h3>
<p>In the sidebar, right-click <i>All</i> under <i>Collection</i> and choose <i>Review collection cleanup…</i> to see references grouped by Windows drive or macOS volume. Reconnect an external disk or check the cloud provider when tracks from that location appear unavailable. Linux offers a read-only review, including for mounted Windows partitions.</p>
<p>On Windows and macOS, select up to 25 eligible references from the local Music folder. Review the paths and the keeper for each duplicate. Confirm the computer and its always-local Music folder, then type the requested phrase, such as <code>MOVE 2</code> for two references. Recordbox verifies a library backup before moving the selected references to Removed Tracks. Audio stays at its original location. To recover a reference within 30 days, right-click <i>All</i> under <i>Collection</i> and choose <i>Restore Removed Tracks…</i>.</p>
"#),
    ("playlists", "Playlists", r#"
<ul>
<li><i>Playlist &gt; Create New Playlist</i>, <i>Create New Intelligent Playlist</i> and <i>Create New Folder</i> create them in the selected folder. Drag tracks onto a playlist to add them; drag inside a playlist (sorted by #) to reorder; drag playlists in the tree to arrange them.</li>
<li>An <b>intelligent playlist</b> lists every track that matches its conditions (BPM, key, genre, rating, date added, My Tag…) and updates itself.</li>
<li><i>Rename Playlist</i> and <i>Delete Playlist</i> act on the playlist selected in the tree.</li>
<li><i>Sort Items</i> puts the playlists of the selected folder in name order.</li>
<li><i>Renumber Track Order</i>: sort the playlist by any column, then make that order the playlist's own.</li>
<li><i>Save Playlist Info</i> writes the selected playlist as an M3U8 file; <i>File &gt; Export Playlists Information</i> writes all of them into a folder.</li>
<li><i>Export Playlist</i> and <i>Export to USB Device…</i> send playlists to a drive (see Exporting).</li>
</ul>
<h3>My Tag</h3>
<p><i>View &gt; My Tag Configuration Window</i> shows your tag categories; tick tags for the selected tracks. With Preferences &gt; Advanced &gt; Browse &gt; Add "My Tag" to the "Comments", a track's tags are also written at the end of its comment as <code>/* Tag / Tag */</code>.</p>
<h3>Related Tracks</h3>
<p>The Related Tracks source lists tracks that go with the loaded track by the rules you tick: BPM range, same or related key, genre, colour, rating, My Tag.</p>
"#),
    ("player", "The player", r#"
<ul>
<li>Choose the destination player or deck in the toolbar above the track list, then double-click a track title or select it and press Return to load it. This also works with the library detached. With the browser and player in one window, you can also drag a track onto the player.</li>
<li><b>Overview</b> (whole track) and <b>enlarged waveform</b>: click or drag to move; Zoom In / Zoom Out / Reset Zoom change the enlarged waveform's scale. Waveform colour (BLUE, RGB, 3Band) is in Preferences &gt; View &gt; Color, Full / Half Waveform in Preferences &gt; View &gt; Display Type.</li>
<li><b>Play/Pause</b> and <b>Cue</b>: Cue while paused sets the cue point; holding it previews from the cue point.</li>
<li><b>Search</b> buttons scan through the track while held; <b>Previous / Next Track</b> load the neighbouring row of the list.</li>
<li><b>Quantize</b> snaps cues and loops to the beat grid (resolution in Preferences &gt; View &gt; recordbox).</li>
<li><b>Beat Jump</b> moves by the chosen number of beats; with Shift, by one beat.</li>
<li>The time display switches between elapsed and remaining.</li>
<li><b>2 Players</b> shows two decks side by side; <b>Simple Player</b> is a compact strip.</li>
</ul>
"#),
    ("cues", "Cues and loops", r#"
<h3>Hot cues</h3>
<p>The number keys <kbd>1</kbd> through <kbd>8</kbd> control hot cues A–H on the active deck. Press an empty pad or its key to set a cue at the playhead. Press it again to jump there, or hold it while paused to preview. To clear one cue, hold <kbd>Shift</kbd> and press its number, or click the × on its pad. For example, <kbd>Shift+1</kbd> clears A. Use <b>Edit &gt; Undo</b> to restore it. Right-click a pad for its colour and comment. Drag a hot cue marker on the enlarged waveform to move it.</p>
<h3>Memory cues</h3>
<p><b>Memory</b> stores the current cue point (or the active loop) as a memory cue. Call Next / Previous Memory Cue step through them; Memory Cue 1–10 call one directly; Delete Memory Cue removes the one at the cue point. The memory list in the side panel shows them with their comments and colours.</p>
<h3>Loops</h3>
<p><b>Loop In</b> then <b>Loop Out</b> sets a manual loop. The beat-loop buttons set an auto loop of that length. <b>Exit/Reloop</b> leaves the loop or returns to the last one. Halve / Double change its length. While a loop plays, Loop In or Loop Out switch to adjusting that edge with the arrow keys. A memory loop can be marked <b>Active Loop</b> so players start looping there by themselves.</p>
<p>Undo and Redo (Edit menu, ⌘Z / ⇧⌘Z) cover cue, loop and grid edits.</p>
"#),
    ("grid", "Beat grid", r#"
<p>Open the GRID panel in the player to correct a grid:</p>
<ul>
<li>Move the whole grid left or right in small steps, or set the first beat of a bar to the current position.</li>
<li>Type a BPM, halve or double it, or tap the tempo.</li>
<li>For tempo changes, edit from the current position onward.</li>
<li>The metronome clicks on the grid so you can hear whether it sits on the beat.</li>
<li>Grid edits have their own undo and redo in the panel. A locked track (Analysis Lock) refuses grid edits.</li>
</ul>
"#),
    ("stems", "Stems", r#"
<p>Recordbox can split a loaded track into vocal, instrument, bass and drum parts with a local separation model. Choose the model in <i>Preferences &gt; Extensions &gt; STEMS</i>. After a successful separation, Recordbox saves a <code>.stems</code> file for reuse with that track and model. The header starts with <code>PDSTEMS1</code>. Desktop library sync handles track edits separately from these audio caches.</p>
<p>Export and Performance decks share saved parts. Quality and speed each save four stereo parts in a separate file. The three-part layout groups bass with instruments. A four-minute track uses about 169 MB per model, or 339 MB for both. Completed results survive a restart. In STEMS preferences, set a shared storage budget (20 GB by default) or choose <i>Keep all saved stems</i>. Copy the stems folder separately when backing up your library.</p>
<p>The part buttons mute, unmute or solo a part. Preferences controls three or four parts, sound quality or speed, and whether muted parts come back when a track is loaded. The same model detects vocal positions, shown above the overview.</p>
"#),
    ("export", "Exporting to USB", r#"
<ol>
<li>Connect a drive formatted as FAT32 (or exFAT / HFS+ for players that read them). It appears under <b>Devices</b>.</li>
<li>Choose <i>Playlist &gt; Export to USB Device…</i>, or select a playlist and <i>Playlist &gt; Export Playlist</i>, or select tracks and <i>Track &gt; Export Track</i>; or drag tracks or playlists onto the device.</li>
<li>Pick the device and the playlists, then Export. Progress is shown in the footer; it can be cancelled.</li>
</ol>
<p>A classic export writes music into <code>/Contents</code>, the catalogs <code>export.pdb</code> and <code>exportExt.pdb</code>, analysis files, artwork and player settings. Select the drive root that contains <code>PIONEER</code>. An interrupted export is rolled back or completed the next time the drive is seen.</p>
<p>If the drive has both classic Device Library and OneLibrary, recordbox updates the classic catalog and protects existing audio, artwork, analysis files and player settings. Players that read OneLibrary continue to show that catalog's contents. For a closed OneLibrary export, right-click the drive and choose <b>Inspect OneLibrary playlists…</b> to read track order with the optional rbox reader. Use AlphaTheta Rekordbox to update the OneLibrary catalog.</p>
<h3>On the device</h3>
<ul>
<li>Under Devices you can browse a drive's playlists and tracks, delete playlists from it, update its playlists from the library, and clean up files nothing refers to.</li>
<li><b>Histories</b>: play histories the players saved are imported as playlists (Preferences &gt; DJ System &gt; Device).</li>
<li>If cues were changed on the players, recordbox offers to take the changes back into the library the next time the drive is connected.</li>
</ul>
<h3>What the players show</h3>
<p>Preferences &gt; DJ System sets the waveform colour, current position and divisions (General), the browse categories and sort options (Category, Sort), and player settings such as Quantize, Auto Cue, Time Mode, Jog Mode and brightness (My Settings). A classic-only drive can receive those player settings. A drive with both catalogs keeps its existing player settings.</p>
"#),
    ("xml", "rekordbox xml", r#"
<p><i>File &gt; Export Collection in xml format</i> writes the whole collection (tracks, cues, grids, playlists) as a rekordbox xml file that rekordbox and other DJ software can import. <i>File &gt; Import Collection from rekordbox xml…</i> reads one. The path remembered as "Imported Library" is in Preferences &gt; Advanced &gt; Browse.</p>
"#),
    ("backup", "Backup and restore", r#"
<h3>Backup Library</h3>
<p><i>File &gt; Library &gt; Backup Library</i> asks "Do you want to back up music files as well?", then where to save. It writes one <code>.zip</code> file holding a consistent copy of the library database (tracks, playlists, cues, grids, My Tags, preferences), the artwork, and — if you said Yes — the music files. A progress window shows what is being written; Cancel leaves no partial file behind.</p>
<h3>Restore Library</h3>
<p><i>File &gt; Library &gt; Restore Library</i> opens a backup file, checks it, and asks before replacing the present library. Before anything is replaced, the present library is copied into the <code>backups</code> folder of the library folder, so a restore can itself be undone. The app keeps running: the restored library is shown as soon as it is in place. Music files in the backup are put back only where the file is missing; a file that exists is never written over.</p>
"#),
    ("prefs", "Preferences", r#"
<p>Open with ⌘, (or recordbox &gt; Preferences…). The field above the categories searches all settings by name. <b>Reset to defaults</b> asks first, and keeps your keyboard shortcuts and column layout.</p>
<ul>
<li><b>View</b> — <i>Display Type</i>: tooltips, list font size and line space, key format (Classic / Alphanumeric). Full / Half Waveform, Beat Count Display. <i>Layout</i>: which sources the Media Browser shows, the playlist palette, cue markers on previews, the playlist search filter, All Tracks and track counts in the playlist tree, phrases and vocal positions on the full waveform. <i>Color</i>: waveform colour. <i>recordbox</i>: time mode, quantize resolution, the other source tiles, My Tag window, column browser, sub-browser.</li>
<li><b>Audio</b> — <i>Configuration</i>: output device, sample rate, buffer size, metronome click sound and volume, preview volume. <i>Input/Output</i>: output channels.</li>
<li><b>Analysis</b> — <i>Track Analysis</i>: choose the Rekordbox-compatible estimator (default) or the earlier recordbox classic estimator. Normal follows a steady tempo and Dynamic follows tempo changes. Either estimator can refine beat timing from audio transients. Normal can additionally use a compatible installed model. The page also controls the BPM range, automatic analysis, and auto gain. These choices apply to new analysis jobs. To replace an existing unlocked grid, select BPM / Grid and choose Track &gt; Analyze Track. <i>CUE Analysis</i>: cues set automatically on first analysis.</li>
<li><b>DJ System</b> — everything written to USB drives: General, Category, Sort, Column, My Settings, Device, Others.</li>
<li><b>Keyboard</b> — every shortcut, by category. Click one and press the new keys; Esc cancels. Profiles can be exported and imported.</li>
<li><b>Advanced</b> — <i>Database</i>: where the library is, backup and restore. <i>Browse</i>: iTunes library file, rekordbox xml, My Tag in comments, Color comments (your names for the eight track colours), folders for Auto Relocate. <i>Others</i>: rekordbox library sync. <i>STEMS</i>: the STEMS function, 3 or 4 parts, sound quality or speed, resetting muted parts on load, the separation model.</li>
</ul>
"#),
    ("trouble", "Troubleshooting", r#"
<ul>
<li><b>No sound</b>: check Preferences &gt; Audio. If the chosen device is unplugged, the system's default output is used until it returns.</li>
<li><b>A track will not load</b>: its file may have moved (see Missing files) or be damaged; the player keeps the previous track loaded.</li>
<li><b>A drive does not appear</b>: open the Devices source to scan again; the drive must be mounted and writable.</li>
<li><b>The grid is wrong</b>: try the other Analysis Mode or a narrower BPM Range and analyse again, or correct it in the GRID panel.</li>
<li><b>Something was removed by mistake</b>: Edit &gt; Undo for cue and grid edits; Track &gt; Restore Removed Tracks… for tracks; File &gt; Library &gt; Restore Library for everything.</li>
</ul>
"#),
];

/// The whole manual as one HTML page.
pub fn html(prefs: &Prefs) -> String {
    let mut s = String::with_capacity(64_000);
    s.push_str("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>recordbox Manual</title><style>");
    s.push_str(":root{--bg:#141414;--fg:#e6e6e6;--dim:#a0a0a0;--card:#1e1e1e;--line:#333;--blue:#3d7be6}@media(prefers-color-scheme:light){:root{--bg:#fafafa;--fg:#1a1a1a;--dim:#555;--card:#fff;--line:#ddd;--blue:#1f5fd0}}");
    s.push_str("body{margin:0;background:var(--bg);color:var(--fg);font:15px/1.55 -apple-system,BlinkMacSystemFont,'Helvetica Neue',sans-serif}main{max-width:820px;margin:0 auto;padding:24px 16px 80px}h1{font-size:28px;margin:8px 0 4px}h2{font-size:21px;margin:40px 0 8px;padding-top:12px;border-top:1px solid var(--line)}h3{font-size:16px;margin:20px 0 4px}a{color:var(--blue)}nav{background:var(--card);border:1px solid var(--line);border-radius:6px;padding:12px 18px;columns:2}nav a{display:block;text-decoration:none;padding:2px 0}code,kbd{font:13px ui-monospace,Menlo,monospace;background:var(--card);border:1px solid var(--line);border-radius:4px;padding:1px 5px}table{border-collapse:collapse;width:100%}td,th{text-align:left;padding:5px 8px;border-bottom:1px solid var(--line)}th{color:var(--dim);font-weight:600}.dim{color:var(--dim)}li{margin:3px 0}@media(max-width:600px){nav{columns:1}}");
    // The logo heads the page (it follows the text colour, light or dark), with the mark as the
    // page icon.
    s.push_str(&format!("</style><link rel=\"icon\" href=\"data:image/svg+xml;utf8,{}\"></head><body><main>", crate::brand::MARK_SVG.replace('"', "'").replace('#', "%23").replace('\n', "")));
    s.push_str(&format!("<h1 class=\"logo\" aria-label=\"recordbox Manual\"><span style=\"display:block;width:280px;max-width:70vw\">{}</span><span style=\"font-size:21px;font-weight:600\">Manual</span></h1>", crate::brand::LOGO_SVG.replace("<svg ", "<svg style=\"width:100%;height:auto;color:var(--fg)\" ")));
    s.push_str(&format!("<p class=\"dim\">Version {}. This page was written by recordbox on this computer; the keyboard shortcuts below are your current ones.</p><nav>", env!("CARGO_PKG_VERSION")));
    for (a, t, _) in CHAPTERS {
        s.push_str(&format!("<a href=\"#{a}\">{}</a>", esc(t)));
    }
    s.push_str("<a href=\"#keys\">Keyboard shortcuts</a></nav>");
    for (a, t, body) in CHAPTERS {
        s.push_str(&format!("<h2 id=\"{a}\">{}</h2>{body}", esc(t)));
        if a == "stems" {
            let folder = crate::db::data_dir_for_inspection().join("stems");
            s.push_str(&format!("<p>recordbox STEMS cache on this computer<br><code>{}</code></p>", esc(&folder.to_string_lossy())));
        }
    }
    s.push_str("<h2 id=\"keys\">Keyboard shortcuts</h2><p>Change them in Preferences &gt; Keyboard. Fixed menu shortcuts: ⌘N Create New Playlist, ⌘E Export to USB Device, ⌘I Property, ⌘F search, ⌘A select all, ⌘Z / ⇧⌘Z Undo / Redo.</p>");
    for cat in crate::prefs_ext::ACTION_CATEGORIES {
        s.push_str(&format!("<h3>{}</h3><table><tr><th style=\"width:65%\">Action</th><th>Shortcut</th></tr>", esc(cat)));
        for (id, label, _, c) in crate::prefs_ext::ACTIONS {
            if c != cat {
                continue;
            }
            let key = prefs.shortcut_for(id).map(|k| format!("<kbd>{}</kbd>", esc(&k.label()))).unwrap_or_else(|| "<span class=\"dim\">not assigned</span>".into());
            s.push_str(&format!("<tr><td>{}</td><td>{key}</td></tr>", esc(label)));
        }
        s.push_str("</table>");
    }
    s.push_str("</main></body></html>");
    s
}

/// Write the manual into the library folder and open it in the browser.
pub fn open(prefs: &Prefs) -> std::io::Result<std::path::PathBuf> {
    let dir = crate::db::data_dir().join("manual");
    std::fs::create_dir_all(&dir)?;
    let p = dir.join("index.html");
    std::fs::write(&p, html(prefs))?;
    let opener = if cfg!(windows) { "explorer.exe" } else if cfg!(target_os = "macos") { "/usr/bin/open" } else { "xdg-open" };
    std::process::Command::new(opener).arg(std::path::absolute(&p)?).spawn()?;
    Ok(p)
}

#[cfg(test)]
mod tests {
    #[test]
    fn manual_covers_every_menu_and_shortcut() {
        let p = crate::app::Prefs::default();
        let h = super::html(&p);
        assert!(h.starts_with("<!doctype html>") && h.ends_with("</html>"));
        for (a, _, _) in super::CHAPTERS {
            assert_eq!(h.matches(&format!("id=\"{a}\"")).count(), 1, "{a}");
            assert!(h.contains(&format!("href=\"#{a}\"")), "{a}");
        }
        for (_, label, _, _) in crate::prefs_ext::ACTIONS {
            assert!(h.contains(&super::esc(label)), "shortcut {label}");
        }
        for name in ["Backup Library", "Restore Library", "Display All Missing Files", "Export Collection in xml format", "Export Playlists Information", "Analysis Lock", "Renumber Track Order", "Save Playlist Info", "Sort Items", "Simple Player", "2 Players", "Sub-Browser Window"] {
            assert!(h.contains(name), "{name}");
        }
        assert!(h.contains("<kbd>Space</kbd>"));
        let cache = crate::db::data_dir_for_inspection().join("stems");
        assert!(h.contains(&format!("<code>{}</code>", super::esc(&cache.to_string_lossy()))));
    }
}
