# USB export compatibility

Choose the drive root that contains `PIONEER`. Check which library your player reads before exporting. For example, AlphaTheta lists the **CDJ-3000X** under OneLibrary. The **CDJ-3000** reverted to Device Library after Pioneer DJ withdrew firmware 3.30. Check the model and installed firmware when comparing results. [AlphaTheta equipment list](https://rekordbox.com/en/support/usb-export/) · [Pioneer DJ firmware notice, 7 January 2026](https://www.pioneerdj.com/en/news/2026/cdj-3000-firmware-ver330-important-notice/)

| Catalogs on the drive | What happens in recordbox |
| --- | --- |
| No recognized device catalog | A classic export can create `PIONEER/rekordbox/export.pdb` and `exportExt.pdb`, copy audio to `Contents` and write track analysis under `PIONEER/USBANLZ`. |
| Classic Device Library | recordbox reads the classic catalogs and can update playlists, tracks and preparation data. Check the cue, playlist-removal and track-deletion options in Device Sync Manager before applying them. |
| OneLibrary only (`exportLibrary.db`) | The optional inspector reads regular playlist order. recordbox blocks classic export to this target. Use AlphaTheta Rekordbox to update its OneLibrary catalog. |
| Both catalogs | recordbox can update the classic catalog or add new classic files. Existing audio, artwork, analysis and player settings are protected. Removing a track from the classic catalog leaves its files on the drive. A player that reads OneLibrary will continue to display the contents of that catalog. |

OneLibrary is the current name for Device Library Plus. The two catalogs can coexist on one drive and can have different playlists and playback histories. In AlphaTheta Rekordbox, **Convert from Device Library** overwrites an existing OneLibrary, including playlists and histories present only there. Before converting, use AlphaTheta’s USB Export Guide to bring back device-only playlists or histories you want to keep. [USB Export Guide](https://cdn.rekordbox.com/files/20251021171528/USB_export_guide_en_251007.pdf) · [Conversion FAQ](https://rekordbox.com/en/support/faq/onelibrary-7/#faq-q700038) · [Playlist and history FAQ](https://rekordbox.com/en/support/faq/onelibrary-7/#faq-q700031) · [Deletion FAQ](https://rekordbox.com/en/support/faq/onelibrary-7/#faq-89312)

## Inspect OneLibrary playlists

Close AlphaTheta Rekordbox and wait for any device export to finish. In **Devices**, right-click the selected USB drive and choose **Inspect OneLibrary playlists…**. On a mixed drive the same button appears in **Sync Manager**. Select a OneLibrary playlist to view its tracks. Select a classic playlist independently to compare the exact sequence of exported audio paths. IDs are local to each catalog. Repeated paths count as separate entries. Hover a track to see its full path.

Install the optional reader once on the computer running recordbox. Packaged builds include the inspection command and use a separately installed Python interpreter with the public `rbox` 0.3.1 library. [rbox package](https://pypi.org/project/rbox/0.3.1/)

**macOS or Linux** with Python 3.12

```sh
python3.12 -m venv "$HOME/.recordbox-onelibrary"
"$HOME/.recordbox-onelibrary/bin/python" -m pip install --only-binary=:all: rbox==0.3.1
```

**Windows PowerShell** with Python 3.12

```powershell
py -3.12 -m venv "$HOME/.recordbox-onelibrary"
& "$HOME/.recordbox-onelibrary/Scripts/python.exe" -m pip install --only-binary=:all: rbox==0.3.1
```

For an existing virtual environment, set `RECORDBOX_ONELIBRARY_PYTHON` to its interpreter’s absolute path before launching recordbox. The CLI takes the drive root above `PIONEER` and prints JSON. The first command lists playlist names and IDs in each catalog. Substitute IDs from that report into the second command.

```sh
recordbox --inspect-onelibrary "/path/to/USB"
recordbox --inspect-onelibrary "/path/to/USB" --playlist 1 --classic-playlist 2
```

Version 1000 is the qualified OneLibrary schema. Inspection copies the OneLibrary database and any sibling classic main catalog into a private temporary directory. Only these copies reach the external reader. recordbox reads each source twice and checks its length, modification time, SHA-256 and current catalog layout before displaying the report. An active USB export journal, SQLite sidecar or changing source stops inspection. If a sidecar remains after closing Rekordbox, keep it with the original database and use a cleanly closed export. The inspector accepts up to 256 MiB per catalog, 50,000 playlist nodes per catalog, 50,000 returned tracks per playlist, and a 16 MiB report.

The comparison covers the order of **resolved track paths**. The public OneLibrary API joins playlist links to existing tracks. A link pointing to a missing track can therefore be omitted, and this report cannot verify orphan entries. Matching paths do not establish audio-file identity or database integrity. The inspector reads no audio or ANLZ and offers no write-back. The public mixed-export test reorders a playlist in a disposable copy using the external writer, observes disagreement with the classic catalog, and restores the order. [Public regression](../packaging/test_public_onelibrary_roundtrip.py)

## What the files contain

The format descriptions below come from public reverse engineering and inspection of published exports. AlphaTheta’s equipment list and export guide document player behavior.

| File | How recordbox uses it |
| --- | --- |
| `PIONEER/rekordbox/export.pdb` | The classic DeviceSQL catalog records track IDs, file and analysis paths, playlist hierarchy and membership. The reader checks table and page chains before returning records. Rewriting supports the known 4,096-byte page layout with 20 main and nine extended table types. A table that exceeds the writer’s index capacity blocks publication. |
| `PIONEER/rekordbox/exportExt.pdb` | The companion DeviceSQL catalog includes My Tag definitions and assignments. It is distinct from the per-track analysis files below. |
| `PIONEER/USBANLZ/…/*.DAT`, `*.EXT`, `*.2EX` | These files use tagged analysis sections. The track’s catalog analysis path leads to its DAT. Its companions can hold extended cues, waveforms, beat timing, phrases or vocal-activity data. Before importing or reusing device analysis on export, recordbox checks the embedded audio path (`PPTH`) against the device track. A disagreement stops that operation. |
| `*.3EX` | A classic export writes DAT, EXT and 2EX. If a track already has a fourth companion on the USB drive, an update keeps its exact bytes and requires matching audio, native BPM and unchanged PQTZ/PQT2 beat-grid sections. An incompatible update stops before publication. USB import can retain the opaque companion with its verified DAT. An explicit whole-track deletion on a classic-only drive also removes its owned companions. |
| `PIONEER/rekordbox/exportLibrary.db` | The separate OneLibrary catalog stays intact during classic edits. Its tracks, playlists and histories need a OneLibrary-capable application to edit. |

Deep Symmetry documents observed DeviceSQL rows, strings, pages and tagged analysis sections. Its Crate Digger project also merged a separate Kaitai format definition for `exportExt.pdb`. [DeviceSQL format](https://djl-analysis.deepsymmetry.org/rekordbox-export-analysis/exports.html) · [ANLZ format](https://djl-analysis.deepsymmetry.org/rekordbox-export-analysis/anlz.html) · [Reviewed extended-catalog format](https://github.com/Deep-Symmetry/crate-digger/pull/36)

A CDJ-2000NXS2 user reported multiple analysis directories for some tracks after using a drive on different players. The report leaves their association unresolved. Keep the catalog references and original directories together when diagnosing a missing cue. [Community report](https://github.com/Deep-Symmetry/crate-digger/issues/11#issuecomment-600459425)

## What has been checked

The public regression cases pair each track’s catalog entry with its analysis files. They copy USB metadata into a temporary directory, compare DAT/EXT/2EX audio-path tags with the catalog, read grids and cues, then apply a single classic rating change. Reopening the device checks the new rating, track references and playlist membership. Hash comparisons cover the other device files, including OneLibrary on the mixed drive. The published trees contain metadata without playable audio. [Public regression tests](../app/src/export/public_device_tests.rs)

A separate compatibility run exercised the classic parser on 14 distinct public main or extended catalog files from rbox, pyrekordbox, rekordcrate and vdj-ui. The largest catalog held 3,886 tracks. These checks cover file and analysis path strings, track lookups, playlists and My Tag references. [DeviceSQL parser](../app/src/export/pdb.rs)

| Source and pinned export | Observed contents |
| --- | --- |
| [pyrekordbox classic export](https://github.com/dylanljones/pyrekordbox/tree/50b4604d98dfd52740aa9e76d80c3a32dfa451ba/.testdata/export) | Six classic tracks, six DAT/EXT/2EX sets and four settings files. |
| [rbox v7 mixed export](https://github.com/dylanljones/rbox/tree/9677e21448fe3a8462111b7df099b5608efcba90/.testdata/RBv7/demo/export) | Two classic tracks, two DAT/EXT/2EX sets and four settings files. A separate read-only inspection of OneLibrary found `property.dbVersion` 1000, two content rows and one playlist with two entries. |

An independent reader generated from Crate Digger’s published Kaitai schema reopened both catalog pairs after the edit. Across 51 numeric and text values per track, the only difference in either export was track 1 changing from zero to one star. The playlist hierarchy and member order, reference IDs and all 28 My Tag category and tag records in each extended catalog matched the originals. [Crate Digger schema used for this check](https://github.com/Deep-Symmetry/crate-digger/blob/7c9d5358d8a3ce663f566380f2dc79ab1393201e/src/main/kaitai/rekordbox_pdb.ksy)

Library Workbench’s independent inspector recognized the four DeviceSQL headers, 24 tagged analysis files and eight settings files across those two exports. Its OneLibrary result reports the filename only. [Workbench format inspector](https://github.com/fantuz/rekordbox-migration-helper/blob/b10e1a45a7a94c0c531d6064eefb79024618cf5e/asset_formats.py) · [OneLibrary schema and reader used for separate inspection](https://github.com/dylanljones/rbox/tree/9677e21448fe3a8462111b7df099b5608efcba90/crates/rbox-onelib)

To repeat the paired regression locally, set `RECORDBOX_TEST_PYREKORDBOX` and `RECORDBOX_TEST_RBOX` to the respective pinned checkout directories above, then run from the recordbox repository root.

```sh
cargo test --manifest-path app/Cargo.toml --locked export::public_device_tests:: -- --ignored --test-threads=1
```

To check a particular player, export a short original track with a known downbeat, memory cue, hot cue and loop to a spare drive. Record the exporting application version, player model, firmware and catalog type. Check playlist visibility, load and playback, then save a cue or playlist on the player and reopen that same drive. A report about mismatched analysis files is most useful with both catalogs, all analysis siblings for the track and their relative directories.

For OneLibrary, the follow-up study compares all 22 tables in a published mixed export and describes a two-track playlist reorder to test on official software and hardware. [OneLibrary round trips](studies/onelibrary-grv6-quality-stems.md#onelibrary-on-a-usb-drive)
