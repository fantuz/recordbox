# Sync the desktop rekordbox library

Open the two arrows at the bottom left of the collection browser or choose **File → Review desktop library sync…**. Check both paths and choose one of the two directions.

| Direction | Source | Destination |
| --- | --- | --- |
| **Import · AlphaTheta Rekordbox → recordbox** | A snapshot of the installed AlphaTheta desktop `master.db` | The active recordbox `library.db` |
| **Write back · recordbox → AlphaTheta Rekordbox** | Selected edits in the active recordbox collection | The installed AlphaTheta desktop library and any analysis files included in the review |

Opening the chooser lets you inspect both paths. On Windows the expected AlphaTheta database is `%APPDATA%\Pioneer\rekordbox\master.db`. On macOS it is `~/Library/Pioneer/rekordbox/master.db`. If the desktop or agent settings identify a different library or a custom analysis-file folder, the write-back pilot stops. The recordbox footer identifies its own active library. **File → Show Library Folder** opens its directory.

## Import into recordbox

Choose **Import · AlphaTheta Rekordbox → recordbox**, then **Import into recordbox** to start the import.

| What Recordbox touches | What happens |
| --- | --- |
| Desktop `master.db` and its write-ahead log | Recordbox takes a temporary private snapshot. A nonempty rollback journal requires closing rekordbox and retrying. It opens the copy read-only with SQLCipher and checks database integrity and the expected schema before importing. |
| The source’s `share` folder and referenced audio | Recordbox can read beat-grid files, check artwork and file paths, and read audio tags or embedded artwork to fill missing details. Only source tracks whose file path resolves to a file on this computer qualify. |
| Recordbox’s data folder | The snapshot is staged here. Confirming sync commits tracks, cues, grids, My Tags, imported playlists and sync state together in Recordbox’s collection. A database write failure rolls back the merge. Reading embedded artwork can also create a local artwork cache entry. |
| USB devices and cloud accounts | Device exports have their own Sync Manager. Desktop library sync makes no provider API requests. An on-demand file can still be hydrated by the operating system when an application reads it. |

### What happens to existing Recordbox work?

- **Tracks** are matched by their recorded file path. Recordbox remembers the source content ID and a fingerprint of source changes. New local tracks are added. A changed source track can replace local metadata, including a rating. An unchanged source generally preserves local edits, while absent cues or a missing grid can be filled in. A path previously removed from Recordbox is skipped.
- **Cues and grids** on a changed track can be replaced by source values. An empty source cue list preserves existing cues. A locked local grid keeps its tempo and timing. Changes to source My Tag assignments can update tags in Recordbox.
- **Playlists and histories** are rebuilt in the imported `rekordbox` folder on each sync. Local edits inside that mirror can be lost. Create your own playlists outside it. Unsupported intelligent-playlist rules are skipped.
- **Analysis** is a separate preference. Adding tracks can queue automatic analysis of locally available tracks that still need it.

For a first test, check the displayed destination and use **File → Library → Backup Library** if it contains work you want to keep. Startup sync is opt-in. To enable it, use **Preferences → Advanced → Others → Import the installed AlphaTheta Rekordbox library at startup**. Close AlphaTheta Rekordbox before taking a snapshot of a busy library. [Desktop reader](../app/src/rbimport.rs) · [Collection merge](../app/src/rbimport/merge.rs) · [Recordbox backup and restore](../app/src/backup.rs)

To try a full import in a fresh Recordbox collection on Windows, open a new PowerShell window in the extracted application folder and run the following. The destination shown in the review should be under the new test directory. Close that PowerShell window after the test to discard the environment override.

```powershell
$env:PREPDECK_DATA = Join-Path $env:TEMP ("Recordbox-test-" + [guid]::NewGuid())
.\recordbox.exe
```

On Linux, run `PREPDECK_DATA="$(mktemp -d)" ./recordbox`. Both examples keep the test Recordbox collection in the temporary folder after Recordbox exits. **The override changes the Recordbox data folder only.** For a write-back test, use an operating-system account with a disposable AlphaTheta desktop library and check the `master.db` path in the chooser.

## Write reviewed changes to AlphaTheta Rekordbox

The write-back pilot targets desktop libraries with a recognized native schema. An unexpected SQL trigger on a table that would be edited blocks the write because it could change additional records. Start with a disposable AlphaTheta profile on the rekordbox version you use. Native application acceptance requires reopening that library in AlphaTheta Rekordbox and checking the selected tracks there.

1. Import from that desktop library to establish a baseline, then edit the tracks in recordbox. The comparison uses saved values, so changes made by Auto Analysis can also appear for review. An older Recordbox collection needs an import with the pilot before edits can be attributed to that native library.
2. Choose **Write back · recordbox → AlphaTheta Rekordbox** and preview the changes. The comparison needs an unambiguous track in the same native library and a saved baseline. For example, if both libraries had two stars at import and you set four in recordbox, the proposed rating is four. If AlphaTheta Rekordbox now has five, the conflict is skipped. If both already show four, there is nothing to apply.
3. Select eligible edits, choose **Review selected changes**, and check the target path and count. Quit rekordbox and its background agent before confirming. The pilot screens for known cloud-related track fields and folder names. Those checks cannot see another linked computer or determine whether Cloud Library Sync is enabled for your account. Use a dedicated local profile for testing and pause cloud synchronization on linked machines.
4. Choose **Back up and apply** for the displayed number of changes. Recordbox checks the native library again under a SQLite writer lock and saves a verified encrypted recovery copy that includes committed changes from the write-ahead log. The result identifies the recovery folder under `native-recovery` beside your active Recordbox `library.db`.

Even a one-rating write-back needs enough free space for a complete database recovery copy. Grid edits can also touch analysis files under the native library’s `share` folder. Keep the entire recovery folder so the affected files and database can be recovered together.

If you want to undo a completed trial through AlphaTheta’s own **Restore Library** command, first make a **Backup Library** ZIP in AlphaTheta Rekordbox. The recordbox native recovery folder supports interrupted write-back and inspection. The **Restore Library** command in Recordbox expects a backup of Recordbox’s own collection.

If the application stops during a grid write-back, close desktop rekordbox and its agents, reopen the arrows, and select **Write back → Review recovery… → Confirm recovery**. Recovery checks the database state and affected file hashes. A verified committed change is kept. If the database did not commit, recovery can restore the corresponding original analysis files. If any recorded state has since changed unexpectedly, keep the recovery folder and investigate before attempting another sync. The small `.recordbox-writeback.lock` file can remain beside `master.db` between operations. The operating system releases its lock when the operation ends.

| Field | What the pilot can review |
| --- | --- |
| Rating | The native zero-to-five-star value for a matched track. |
| BPM | The track's numeric BPM. If that number tracks a constant-tempo native grid, review and select the corresponding grid change together with the BPM change. |
| Key | A change to an existing, unambiguous native key entry. An unknown key or ambiguous mapping is skipped. |
| Cues | Change comments and colors, reassign an existing hot cue to another pad at the same time, or remove an existing cue when its native mirrors can stay consistent. Referenced hot-cue banks and unsupported sidecar cue data can block an edit. Creating cues and changing their start or loop times require native seek information that Recordbox does not generate. |
| Beat grid | Shift a supported grid in whole milliseconds or change which beat starts the bar. A constant-tempo grid can also be retimed when a detailed waveform supplies timing evidence and the track has no dependent cues or phrases. A track with only a DAT beat grid can keep that layout. Unknown companion data or edits that add or remove tempo segments block retiming. |

For a whole-grid nudge, choose **Adjust whole Beatgrid** in the deck’s grid editor, then click the left or right arrow. A plain click moves every beat by 1 ms. The desktop writer preserves each beat’s BPM and position in the bar. On a track with a validated `PQT2` revision `0x02000002`, it moves the timestamps in the DAT and clears the secondary table in the EXT, retaining its header. Subsequent nudges keep that table empty. A track with a supported `.3EX` can use this operation when the embedding identifies the track or its unassigned ID can be verified through a unique native file registration. The preview still holds tempo changes and bar rotations for these embeddings.

When a recognized `.3EX` has an unassigned track ID, a cue comment or color edit requires independent native file registration matching the track, path, size and checksum. Pad reassignments and removals are held for these files. An unsupported `PQT2` revision is identified in the timing review so the edit can be completed in AlphaTheta Rekordbox.

For newly placed or retimed cues and more complex tempo maps, **File → Export Collection in xml format** writes cue positions and tempo points to a collection XML file. Review a manual XML import in a disposable AlphaTheta profile and check the resulting cues and downbeats before adopting it in your main collection. [Recordbox XML exporter](../app/src/xmlexport.rs)

The pilot draws on Library Workbench’s native backup writer for per-row update counters, the library-wide change counter and the relationship between cue rows and the serialized cue list. Workbench performs those edits in an exported backup copy. A Recordbox write-back targets the installed library identified in its own review. [Workbench native writer](https://github.com/fantuz/rekordbox-migration-helper/blob/b10e1a45a7a94c0c531d6064eefb79024618cf5e/native_export.py) · [Recordbox field checks](../app/src/rbimport/writeback/mod.rs) · [Cue edits](../app/src/rbimport/writeback/cues.rs) · [Grid edits](../app/src/rbimport/writeback/grid.rs)

## Locate separated STEMS

After Recordbox separates a track, it caches the four stereo parts in a `.stems` file. `PDSTEMS1` is the first eight bytes of that file. Open **Preferences → Extensions → STEMS** and copy the **recordbox STEMS cache** path. The built-in manual also shows this directory in its **Stems** chapter.

| System | Default Recordbox STEMS folder |
| --- | --- |
| Windows | `%APPDATA%\Recordbox\stems` |
| macOS | `~/Library/Application Support/Recordbox/stems` |
| Linux | `$XDG_DATA_HOME/Recordbox/stems`, normally `~/.local/share/Recordbox/stems` |

Setting `PREPDECK_DATA` puts the cache in the `stems` subfolder of that directory. Filenames have the form `<24 hex characters>-<model>.stems`. Recordbox hashes the audio path, file size and modification time in seconds for the prefix. The file header stores the format and frame count. Library Workbench can inspect its structure in **Library lab → Formats & STEMS**. [Recordbox cache code](../app/src/stems/mod.rs) · [Workbench format inspector](https://github.com/fantuz/rekordbox-migration-helper/blob/b10e1a45a7a94c0c531d6064eefb79024618cf5e/docs/FORMATS-AND-STEMS.md)

### How much separation is saved?

| Analysis setting | Models run | Saved per track | Size for four minutes |
| --- | ---: | --- | ---: |
| Prioritize speed | 1 | Four stereo parts in `-htdemucs.stems` | 169.34 MB |
| Prioritize sound quality | 4 | Four stereo parts in `-htdemucs_ft.stems` | 169.34 MB |
| Both, run separately | 5 in total | Two files for the same track | 338.69 MB |

Each file stores drums, bass, instruments and vocals as 16-bit PCM at 44,100 stereo frames per second. One minute takes 42.34 MB plus a 16-byte header per file. For comparison, four minutes of stereo 16-bit/44.1 kHz WAV audio takes about 42.34 MB, and a 320 kb/s MP3 takes about 9.6 MB, excluding tags. Track duration determines the size of the separation. Downloaded model weights are shared between tracks and live in the separate `models` directory.

Export and Performance decks use the same cache. The 3-part layout groups bass with instruments under **INST**. Both layouts save all four parts. Completed files survive track changes, switching models, and closing Recordbox. Unloading a track from its last deck before separation finishes cancels the job. On a later load, Recordbox needs the same model, source path, size and modification time to reuse the stored audio.

The default **50 GB** budget is shared across tracks and models. It fits about 295 four-minute tracks prepared with one model, or 147 with both. In **Preferences → Extensions → STEMS**, enter a custom budget or choose **Keep all saved stems** to remove the fixed cache-size limit. Existing saved budgets, including 20 GB and zero, carry forward. Choose **Use 50 GB default** to adopt the new value.

Every save also reserves **10% of the cache volume’s total capacity**. On a 200 GB drive with 42 GB available, at most 22 GB can be added while maintaining that reserve. Other applications and files on that drive count toward occupancy. recordbox checks for enough room to write the complete new file before replacing an existing result. If space is insufficient, the finished separation remains playable on the loaded deck, and the cache panel explains why saving failed. Changing the budget alone leaves existing files in place.

For a backup, copy the `stems` folder separately from the Recordbox library backup. When restoring to another installation, preserve the source audio paths, file sizes and modification times so Recordbox can recognize the saved work.

AlphaTheta’s own per-track STEMS storage and a compatible import format remain unverified. AlphaTheta documents `.rbsc` as a captured DRUMS sample. A useful native sample for whole-track separation starts with a short self-created track in a disposable AlphaTheta library. Record the rekordbox version, OS, mode and STEMS setting, inventory the library and cache before and after separation, then check whether any generated file persists after quitting and reopening rekordbox. Ratings, cues, BPM, key and beat-grid write-back can be tested independently of STEMS. [AlphaTheta sample capture in Workbench](https://github.com/fantuz/rekordbox-migration-helper/blob/b10e1a45a7a94c0c531d6064eefb79024618cf5e/docs/DISCOVERY-ASSETS-HISTORY.md) · [Workbench STEMS evidence](https://github.com/fantuz/rekordbox-migration-helper/blob/b10e1a45a7a94c0c531d6064eefb79024618cf5e/docs/STEMS-WORKFLOWS.md)

## Sync a USB drive

**Device Sync Manager** works on a selected USB drive. A classic export can write device catalogs, audio, artwork and analysis files. The review lists previously managed playlists selected for removal. On a classic-only drive, settings can allow replacing device cues or removing tracks and their files. On a mixed drive, catalog changes apply to the classic library and existing shared files are protected. A OneLibrary-only drive requires AlphaTheta Rekordbox. [USB compatibility, tested exports and player models](USB-COMPATIBILITY.md) · [Device Sync Manager](../app/src/device_ext.rs)

## How the formats compare with Library Workbench

| Source or destination | Recordbox | Library Workbench |
| --- | --- | --- |
| Desktop v6/v7 `master.db` | Imports supported records from a private snapshot. The manual write-back pilot checks the native schema, identity and baseline before offering edits. | Captures the source for inspection, including missing and cloud file references. Its snapshot reader handles WAL and rollback-journal evidence. |
| Desktop v5 and earlier `.edb` | Use a collection XML export for interchange. | Detects legacy database files and inventories them. Use XML for track-level analysis. |
| Collection XML | Imports and exports tracks, playlist links, supported cues and tempo information. | Inspects XML alone, or pairs it with a native backup for review and edited export. |
| Backup ZIP | Creates and restores Recordbox’s own backups, identified by their manifest and `library.db`. | Opens AlphaTheta’s Backup Library ZIP and can prepare an edited native backup copy. |
| Classic USB Device Library | Reads and writes supported DeviceSQL records in `export.pdb` and `exportExt.pdb` and handles associated audio and ANLZ files. | Inspects catalog headers, settings and selected ANLZ evidence. Device record decoding is outside its inspector’s scope. |
| OneLibrary / Device Library Plus | Detects `exportLibrary.db`. Classic exports on a mixed drive update only the classic catalog. | Recognizes the separate OneLibrary catalog by filename. Its records remain undecoded in both applications. |

Workbench sources for this comparison were checked at commit `b10e1a4`. [Desktop analyzer](https://github.com/fantuz/rekordbox-migration-helper/blob/b10e1a45a7a94c0c531d6064eefb79024618cf5e/analyzer.py) · [Native backup exporter](https://github.com/fantuz/rekordbox-migration-helper/blob/b10e1a45a7a94c0c531d6064eefb79024618cf5e/native_export.py) · [Device-format inspector](https://github.com/fantuz/rekordbox-migration-helper/blob/b10e1a45a7a94c0c531d6064eefb79024618cf5e/asset_formats.py)

In a Linux GUI test with a generated encrypted library, a track had two proposed changes. Selecting only its rating changed two stars to four and preserved the original 120 BPM. Library Workbench independently reopened the recovery and edited databases and compared the schema and all seven active tables covered by its verifier against the expected rating edit. Missing-source tests exercised both directions. [Workbench export verifier](https://github.com/fantuz/rekordbox-migration-helper/blob/b10e1a45a7a94c0c531d6064eefb79024618cf5e/export_verification.py)

A separate test used a disposable copy of the published pyrekordbox v6 encrypted desktop catalog. It matched the original analysis files to their registered hashes and relocated fixture paths into the test directory. Applying a rating, a 10 ms grid shift and a change of downbeat updated the native DAT and EXT files, their registered hashes and sizes, and the selected rating. Reopening the recovery database confirmed the previous rating. This exercises native library data through Recordbox. Opening the edited collection in the AlphaTheta application remains a separate acceptance test. [Public v6 fixture](https://github.com/dylanljones/pyrekordbox/tree/50b4604d98dfd52740aa9e76d80c3a32dfa451ba/.testdata/rekordbox%206) · [Native write-back tests](../app/src/rbimport/writeback/integration_tests.rs)

On a disposable copy of rbox’s public encrypted v7 demo library, Recordbox changed Hot Cue A’s Unicode comment and color and the track’s rating. The native cue identity, seek metadata and all four analysis files were preserved. The test also reopened the recovery database and repeated the preview. [Public v7 fixture](https://github.com/dylanljones/rbox/tree/9677e21448fe3a8462111b7df099b5608efcba90/.testdata/RBv7/demo/rekordbox) · [Native cue tests](../app/src/rbimport/writeback/cues.rs)

The grid tests inspected six desktop analysis sets from v6 and 32 from v7. Of those, 12 held nonempty grids. All 29 v7 embedding companions matched a known layout. On the ten sampled tracks with both an embedding and a nonempty grid, one embedding field matched the track’s BPM in every case. The pilot permits verified whole-grid shifts for compatible analysis layouts and checks ownership of the embedding. BPM edits require separate evidence. Cue labels and colors have separate eligibility checks, including a match between the embedding’s track ID and the native cue owner. [Grid regressions and public fixture check](../app/src/rbimport/writeback/grid.rs) · [Cue eligibility](../app/src/rbimport/writeback/cues.rs)

In two desktop captures, shifting the entire grid one click right moved every stored timestamp by exactly 1 ms. A four-beat sample had no secondary table. On a 725-beat track, Rekordbox cleared all 725 secondary entries and preserved the cue, waveform, phrase and embedding bytes. The four analysis files collected after quitting matched the files captured after the edit with Rekordbox still open. In a separate capture, placing a new grid at the playhead moved the first beat by 14 ms and the others by 24 ms, and changed one byte in the phrase header. That edit falls outside the uniform-nudge case. The exact Rekordbox build was unspecified. USB exports have their own checks. [Replay test](../app/src/rbimport/grid_oracle.rs)

Generated desktop-sync tests read committed rows from an encrypted WAL, verify source-file preservation, reject overlapping snapshot paths and exercise repeated imports, local edits, empty source cues, locked grids and preservation of a user folder that reuses a deleted mirror ID. A forced database-write error exercises rollback of the destination tracks, mirror and sync state. Other tests cover encrypted SQLite/WAL reads and classic USB exports. The installed-library check runs separately with a local installation. Hardware acceptance on a particular player, or genuine-library compatibility with a specific rekordbox build, needs separate testing. Workbench reports v7.2.19 evidence and lists genuine v6 fixture validation as pending. [Recordbox database tests](../app/src/db/sqlcipher_tests.rs) · [USB regression tests](../app/src/export/usb_fix_test.rs) · [Workbench compatibility](https://github.com/fantuz/rekordbox-migration-helper/blob/b10e1a45a7a94c0c531d6064eefb79024618cf5e/docs/COMPATIBILITY.md) · [Workbench format evidence](https://github.com/fantuz/rekordbox-migration-helper/blob/b10e1a45a7a94c0c531d6064eefb79024618cf5e/docs/FORMAT-SOURCES.md)
