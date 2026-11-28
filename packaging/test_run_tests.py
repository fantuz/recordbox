"""Application CI requires a complete passing suite, with no failure allowance."""
import unittest

from run_tests import accepts_suite


LOG = """   Finished `test` profile [optimized + debuginfo] target(s) in 0.24s
     Running unittests src/main.rs (app/target/debug/deps/recordbox-123)

running 3 tests
test backup::tests::backup_restore_round_trip ... ok
test export::usb_fix_test::a1_a_row_too_big_for_a_page_is_an_error_not_a_panic ... ok
test export::usb_test::recover_real_device ... ignored, requires a manually supplied device

test result: ok. 2 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 1.00s

"""

NATIVE_OUTPUT = """test db::sqlcipher_tests::encrypted_wal_reads_backup_reopen_and_wrong_key ... 2026-11-19 16:09:23.828: ERROR CORE sqlcipher_page_cipher: hmac check failed for pgno=1
2026-11-19 16:09:23.828: ERROR CORE sqlite3Codec: error decrypting page 1 data: 1
2026-11-19 16:09:23.828: ERROR CORE sqlcipher_codec_ctx_set_error: 1 lock=0
ok"""

NATIVE_LOG = LOG.replace("test backup::tests::backup_restore_round_trip ... ok", NATIVE_OUTPUT)


class ApplicationSuiteTests(unittest.TestCase):
    def test_complete_passing_suite_and_explicit_ignored_tests_are_accepted(self):
        self.assertTrue(accepts_suite(LOG, 0))
        self.assertTrue(accepts_suite(LOG.replace(", requires a manually supplied device", ""), 0))
        all_passed = LOG.replace("ignored, requires a manually supplied device", "ok")
        all_passed = all_passed.replace("2 passed; 0 failed; 1 ignored", "3 passed; 0 failed; 0 ignored")
        self.assertTrue(accepts_suite(all_passed, 0))

    def test_expected_panic_test_uses_valid_libtest_output(self):
        self.assertTrue(accepts_suite(LOG.replace(
            "backup::tests::backup_restore_round_trip ... ok",
            "backup::tests::rejects_invalid_input - should panic ... ok"), 0))

    def test_native_diagnostics_can_interleave_a_serial_test_completion(self):
        self.assertTrue(accepts_suite(NATIVE_LOG, 0))
        self.assertTrue(accepts_suite(NATIVE_LOG.replace("... 2026-11-19", "... \n2026-11-19"), 0))
        self.assertTrue(accepts_suite(NATIVE_LOG.replace("lock=0\nok", "lock=0\nerror: expected wrong key\nok"), 0))

    def test_a_nested_filtered_child_summary_must_stay_out_of_the_full_suite_log(self):
        name = "stem_export::tests::offdeck_export_reads_the_model_cache_and_rejects_incomplete_parts"
        outer = LOG.replace("backup::tests::backup_restore_round_trip", name)
        child = (f"\nrunning 1 test\ntest {name} ... ok\n\n"
                 "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; "
                 "972 filtered out; finished in 0.01s\n\n")
        self.assertFalse(accepts_suite(outer.replace(f"test {name} ... ok", f"test {name} ... {child}ok"), 0))
        self.assertTrue(accepts_suite(outer, 0))

    def test_native_test_still_requires_its_own_completion_before_next_test_or_summary(self):
        missing_completion = NATIVE_LOG.replace("lock=0\nok", "lock=0")
        self.assertFalse(accepts_suite(missing_completion, 0))
        # A later test's completion cannot finish the earlier incomplete native test.
        self.assertFalse(accepts_suite(missing_completion.replace(
            "export::usb_fix_test::a1_a_row_too_big_for_a_page_is_an_error_not_a_panic ... ok",
            "export::usb_fix_test::a1_a_row_too_big_for_a_page_is_an_error_not_a_panic ... \nok"), 0))
        truncated = NATIVE_LOG[:NATIVE_LOG.index("\nok\n")]
        self.assertFalse(accepts_suite(truncated, 0))
        self.assertFalse(accepts_suite(truncated + "\n" + NATIVE_LOG[NATIVE_LOG.index("test result:"):] + "\nok\n", 0))

    def test_failed_or_duplicate_native_completion_is_not_accepted(self):
        failed = NATIVE_LOG.replace("lock=0\nok", "lock=0\nFAILED")
        self.assertFalse(accepts_suite(failed, 0))
        self.assertFalse(accepts_suite(NATIVE_LOG, 101))
        duplicate = NATIVE_LOG.replace(
            "export::usb_fix_test::a1_a_row_too_big_for_a_page_is_an_error_not_a_panic",
            "db::sqlcipher_tests::encrypted_wal_reads_backup_reopen_and_wrong_key")
        self.assertFalse(accepts_suite(duplicate, 0))

    def test_no_failures_are_accepted_including_former_pdb_exception(self):
        for name in ("backup::tests::backup_restore_round_trip",
                     "export::usb_fix_test::a1_a_row_too_big_for_a_page_is_an_error_not_a_panic"):
            log = LOG.replace(f"test {name} ... ok", f"test {name} ... FAILED")
            log = log.replace("test result: ok. 2 passed; 0 failed", "test result: FAILED. 1 passed; 1 failed")
            for code in (0, 101):
                with self.subTest(name=name, code=code):
                    self.assertFalse(accepts_suite(log, code))

    def test_crash_or_error_exit_rejects_even_a_passing_summary(self):
        for code in (-11, 1, 101):
            with self.subTest(code=code):
                self.assertFalse(accepts_suite(LOG, code))
        self.assertFalse(accepts_suite(LOG + "error: could not compile recordbox\n", 0))

    def test_filtered_measured_or_empty_suite_is_rejected(self):
        for old, new in (("0 filtered out", "1 filtered out"),
                         ("0 measured", "1 measured"),
                         ("2 passed", "0 passed")):
            with self.subTest(new=new):
                self.assertFalse(accepts_suite(LOG.replace(old, new), 0))

    def test_missing_or_truncated_runner_output_is_rejected(self):
        self.assertFalse(accepts_suite(LOG.split("test result:")[0], 0))
        self.assertFalse(accepts_suite(LOG.replace("running 3 tests\n", ""), 0))
        self.assertFalse(accepts_suite(LOG.replace("running 3 tests", "running 4 tests"), 0))
        self.assertFalse(accepts_suite(LOG.replace("test backup::tests::backup_restore_round_trip ... ok\n", ""), 0))
        self.assertFalse(accepts_suite(LOG.replace("; finished in 1.00s", ""), 0))

    def test_forged_duplicate_or_mismatched_results_are_rejected(self):
        self.assertFalse(accepts_suite(LOG.replace(
            "backup::tests::backup_restore_round_trip",
            "export::usb_fix_test::a1_a_row_too_big_for_a_page_is_an_error_not_a_panic"), 0))
        self.assertFalse(accepts_suite(LOG.replace("2 passed; 0 failed; 1 ignored", "1 passed; 0 failed; 2 ignored"), 0))
        self.assertFalse(accepts_suite(LOG + LOG, 0))


if __name__ == "__main__":
    unittest.main()
