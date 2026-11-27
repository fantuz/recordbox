"""Read operator-selected playlists from private catalog snapshots using optional rbox."""
import json
import sys
from importlib import metadata
from pathlib import Path


def playlists(library):
    nodes = library.get_playlists()
    if len(nodes) > 50000:
        raise ValueError("This reader supports up to 50,000 playlist nodes per catalog.")
    return [
        {"id": node.id, "parent_id": node.parent_id, "name": node.name, "kind": int(node.attribute)}
        for node in sorted(nodes, key=lambda n: (n.parent_id, n.seq, n.id))
    ]


def tracks(library, selected, nodes):
    if selected is None:
        return None
    node = next((n for n in nodes if n["id"] == selected), None)
    if node is None or node["kind"] != 0:
        raise ValueError(f"Select an existing regular playlist (requested ID {selected}).")
    rows = library.get_playlist_contents(selected)
    if len(rows) > 50000:
        raise ValueError("This reader supports up to 50,000 playable entries per playlist.")
    return [{"id": row.id, "title": row.title or "", "path": row.path} for row in rows]


def read(folder, one_id, classic_id):
    from rbox import DeviceLibrary, OneLibrary
    one = OneLibrary.from_path(str(folder / "exportLibrary.db"))
    properties = one.get_properties()
    if not properties or any(prop.db_version != 1000 for prop in properties):
        raise ValueError("Unsupported OneLibrary property.dbVersion (this reader supports 1000).")
    nodes = playlists(one)
    one_tracks = tracks(one, one_id, nodes)
    classic_nodes = classic_tracks = comparison = None
    classic_file = folder / "export.pdb"
    if classic_file.exists():
        classic = DeviceLibrary.from_paths(classic_file)
        classic_nodes = playlists(classic)
        classic_tracks = tracks(classic, classic_id, classic_nodes)
    elif classic_id is not None:
        raise ValueError("This export has no classic catalog to compare.")
    if classic_tracks is not None:
        one_paths = [t["path"] for t in one_tracks]
        classic_paths = [t["path"] for t in classic_tracks]
        if not all(one_paths) or not all(classic_paths):
            comparison = "indeterminate"
        elif one_paths == classic_paths:
            comparison = "same_path_order"
        else:
            comparison = "different_path_order"
    return dict(db_version=1000, one_playlists=nodes, classic_playlists=classic_nodes,
                one_tracks=one_tracks, classic_tracks=classic_tracks, comparison=comparison)


def main():
    try:
        if metadata.version("rbox") != "0.3.1":
            raise ImportError("The reader requires rbox==0.3.1.")
        folder, *ids = sys.argv[1:]
        selected, classic = (None if value == "-" else int(value) for value in ids)
        result = json.dumps(read(Path(folder), selected, classic), ensure_ascii=True)
        if len(result) > 16 * 1024 * 1024:
            raise ValueError("This report exceeds the 16 MiB output limit.")
        print(result)
    except (metadata.PackageNotFoundError, ImportError) as error:
        sys.exit(f"Install rbox==0.3.1 in the OneLibrary helper environment. {error}")
    except Exception as error:
        sys.exit(f"Could not inspect the catalog snapshot: {error}")


if __name__ == "__main__":
    main()
