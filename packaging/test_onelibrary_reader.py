"""Public API contract and conservative reporting without an optional installation."""
import importlib.util
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location(
    "onelibrary_reader", Path(__file__).resolve().parents[1] / "app/src/onelibrary_reader.py"
)
reader = importlib.util.module_from_spec(spec)
spec.loader.exec_module(reader)


class OneLibraryReaderTests(unittest.TestCase):
    def test_unqualified_version_declines_before_querying_playlists(self):
        for properties in ([], [SimpleNamespace(db_version=1001)],
                           [SimpleNamespace(db_version=1000), SimpleNamespace(db_version=999)]):
            one = SimpleNamespace(get_properties=lambda: properties)
            rbox = SimpleNamespace(DeviceLibrary=None, OneLibrary=SimpleNamespace(from_path=lambda path: one))
            with patch.dict(sys.modules, rbox=rbox), self.assertRaisesRegex(ValueError, "dbVersion"):
                reader.read(Path("private"), None, None)

    def test_comparison_uses_literal_paths_in_returned_order_and_preserves_duplicates(self):
        node = SimpleNamespace(id=1, parent_id=0, name="Mix", attribute=0, seq=1)
        def catalog(content_ids, paths):
            rows = [SimpleNamespace(id=id_, path=path, title="same title") for id_, path in zip(content_ids, paths)]
            return SimpleNamespace(get_playlists=lambda: [node], get_playlist_contents=lambda id_: rows,
                                   get_properties=lambda: [SimpleNamespace(db_version=1000)])
        one = catalog([3, 3, 7], ["/A", "/A", "/B"])
        with tempfile.TemporaryDirectory() as tmp:
            folder = Path(tmp)
            (folder / "export.pdb").touch()
            for classic_paths, result in [
                (["/A", "/A", "/B"], "same_path_order"), (["/A", "/B"], "different_path_order"),
                (["/A", "/B", "/A"], "different_path_order"), (["/a", "/A", "/B"], "different_path_order"),
                (["/A", "", "/B"], "indeterminate"),
            ]:
                classic = catalog([1, 2, 8], classic_paths)
                rbox = SimpleNamespace(OneLibrary=SimpleNamespace(from_path=lambda path: one),
                                       DeviceLibrary=SimpleNamespace(from_paths=lambda path: classic))
                with patch.dict(sys.modules, rbox=rbox):
                    self.assertEqual(reader.read(folder, 1, 1)["comparison"], result)
                    with self.assertRaisesRegex(ValueError, "regular playlist"):
                        reader.read(folder, 99, None)
