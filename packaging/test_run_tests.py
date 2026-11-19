"""The test baseline permits one specific PDB panic, never arbitrary failures."""
import unittest

from run_tests import KNOWN, accepts_baseline


LOG = f"""   Finished `test` profile [optimized + debuginfo] target(s) in 0.24s
     Running unittests src/main.rs (app/target/debug/deps/recordbox-123)

running 3 tests
test backup::tests::backup_restore_round_trip ... ok
test export::pdb::tests::database_roundtrip ... ok
test {KNOWN} ... FAILED

failures:

---- {KNOWN} stdout ----

thread '{KNOWN}' (12345) panicked at src/export/pdb.rs:350:24:
attempt to subtract with overflow
stack backtrace:
   0: rust_begin_unwind

failures:
    {KNOWN}

test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.00s

error: test failed, to rerun pass `--bin recordbox`
"""


class TestBaselineTests(unittest.TestCase):
    def test_only_documented_pdb_failure_is_accepted(self):
        self.assertTrue(accepts_baseline(LOG, 101))
        self.assertTrue(accepts_baseline(LOG.replace("src/export/pdb.rs", r"C:\work\app\src\export\pdb.rs"), 101))

    def test_changed_panic_source_or_message_is_rejected(self):
        for old, new in (("src/export/pdb.rs", "src/export/usb_fix_test.rs"),
                         ("attempt to subtract with overflow", "index out of bounds")):
            with self.subTest(new=new):
                self.assertFalse(accepts_baseline(LOG.replace(old, new), 101))

    def test_another_failed_test_is_rejected(self):
        log = LOG.replace("test export::pdb::tests::database_roundtrip ... ok",
                          "test export::pdb::tests::database_roundtrip ... FAILED")
        self.assertFalse(accepts_baseline(log.replace("2 passed; 1 failed", "1 passed; 2 failed"), 101))

    def test_crash_or_missing_summary_is_rejected(self):
        for code in (-11, 1, 0):
            with self.subTest(code=code):
                self.assertFalse(accepts_baseline(LOG, code))
        self.assertFalse(accepts_baseline(LOG.split("test result:")[0], 101))

    def test_all_passed_means_the_baseline_needs_removing(self):
        log = LOG.replace(f"test {KNOWN} ... FAILED", f"test {KNOWN} ... ok")
        log = log.replace("test result: FAILED. 2 passed; 1 failed", "test result: ok. 3 passed; 0 failed")
        self.assertFalse(accepts_baseline(log, 0))

    def test_filtered_or_incomplete_suite_is_rejected(self):
        self.assertFalse(accepts_baseline(LOG.replace("0 filtered out", "1 filtered out"), 101))
        self.assertFalse(accepts_baseline(LOG.replace("2 passed", "0 passed"), 101))
        self.assertFalse(accepts_baseline(LOG + "error: could not compile recordbox\n", 101))

    def test_panic_outside_failure_block_is_rejected(self):
        panic = f"thread '{KNOWN}' (12345) panicked at src/export/pdb.rs:350:24:\nattempt to subtract with overflow\n"
        self.assertFalse(accepts_baseline(LOG.replace(panic, "different failure\n") + panic, 101))


if __name__ == "__main__":
    unittest.main()
