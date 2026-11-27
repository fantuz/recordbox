"""Opt-in cross-reader regression on the public mixed rbox RBv7 demo export.

Set RECORDBOX_TEST_RBOX to the public fixture checkout, RECORDBOX_TEST_BINARY to
an application executable, and RECORDBOX_ONELIBRARY_PYTHON to Python with
rbox==0.3.1 installed. The external writer only sees a temporary copy.
"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


REQUIRED = ('RECORDBOX_TEST_RBOX', 'RECORDBOX_TEST_BINARY', 'RECORDBOX_ONELIBRARY_PYTHON')
CATALOG = 'rekordbox/exportLibrary.db'
PUBLIC_CATALOG_SHA256 = '0a7e8c8078ebf1ec318410ec31f0d9f4992e43e0dd20eb6d6847a5e70b6b4aff'


def file_hashes(root):
    return {file.relative_to(root).as_posix(): hashlib.sha256(file.read_bytes()).hexdigest()
            for file in root.rglob('*') if file.is_file()}


@unittest.skipUnless(all(os.environ.get(name) for name in REQUIRED), 'requires public rbox fixture, recordbox and rbox 0.3.1')
class PublicOneLibraryTests(unittest.TestCase):
    def test_classic_and_onelibrary_orders_after_external_edit_and_undo(self):
        checkout = Path(os.environ['RECORDBOX_TEST_RBOX'])
        binary = Path(os.environ['RECORDBOX_TEST_BINARY']).absolute()
        python = Path(os.environ['RECORDBOX_ONELIBRARY_PYTHON']).absolute()
        source = checkout / '.testdata/RBv7/demo/export/PIONEER'
        baseline = file_hashes(source)
        self.assertEqual(baseline[CATALOG], PUBLIC_CATALOG_SHA256)
        self.assertEqual(len(baseline), 14)

        with tempfile.TemporaryDirectory() as directory:
            drive = Path(directory) / 'USB démo'
            tree = drive / 'PIONEER'
            shutil.copytree(source, tree)
            for position, ids, comparison in ((None, [1, 2], 'same_path_order'),
                                              (1, [2, 1], 'different_path_order'),
                                              (2, [1, 2], 'same_path_order')):
                with self.subTest(destination=position):
                    if position is not None:
                        # Use a separate writer process so all handles close before inspection.
                        subprocess.run([python, '-I', '-c',
                                        'from rbox import OneLibrary\nimport sys\n'
                                        'OneLibrary(sys.argv[1]).move_playlist_content(1, 2, int(sys.argv[2]))',
                                        tree / 'rekordbox', str(position)], check=True, timeout=30)
                    before_read = file_hashes(tree)
                    result = subprocess.run([binary, '--inspect-onelibrary', drive, '--playlist', '1',
                                             '--classic-playlist', '1'], check=True, capture_output=True,
                                            timeout=90)
                    report = json.loads(result.stdout)
                    self.assertEqual(report['db_version'], 1000)
                    self.assertEqual(report['comparison'], comparison)
                    self.assertEqual([track['id'] for track in report['one_tracks']], ids)
                    self.assertEqual([track['id'] for track in report['classic_tracks']], [1, 2])
                    self.assertEqual(file_hashes(tree), before_read)
                    self.assertEqual({k: v for k, v in before_read.items() if k != CATALOG},
                                     {k: v for k, v in baseline.items() if k != CATALOG})
            self.assertEqual(file_hashes(source), baseline)


if __name__ == '__main__':
    unittest.main()
