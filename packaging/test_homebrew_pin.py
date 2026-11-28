import contextlib
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import homebrew_pin as pin


class PinTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        self.origin = self.root / "origin.git"
        self.work = self.root / "work"
        subprocess.run(["git", "init", "-q", "--bare", "-b", "main", self.origin], check=True)
        subprocess.run(["git", "init", "-q", "-b", "main", self.work], check=True)
        cwd = Path.cwd()
        self.addCleanup(os.chdir, cwd)
        os.chdir(self.work)
        self.git("config", "user.name", "Test")
        self.git("config", "user.email", "test@example.invalid")
        self.git("remote", "add", "origin", str(self.origin))
        self.tag = "v1.2.3"
        self.formula = Path(pin.FORMULA)
        self.formula.parent.mkdir()
        self.original = ('class Recordbox < Formula\n  url "https://github.com/fantuz/recordbox.git",\n'
                         '      tag:      "v1.2.0",\n      revision: "' + "1" * 40 + '"\nend\n')
        self.formula.write_text(self.original)
        Path("VERSION").write_text("1.2.3\n")
        self.git("add", ".")
        self.git("commit", "-qm", "Publish source")
        self.src = self.git("rev-parse", "HEAD")
        self.git("tag", "-a", self.tag, "-m", self.tag)
        self.git("push", "origin", "main", self.tag)
        self.release = {"tag_name":self.tag, "draft":False, "prerelease":False, "published_at":"today",
                        "target_commitish":self.src, "body":f"<!-- recordbox-release-source: {self.src} -->"}
        subjects = [n for _, _, _, n in pin.release_archives(self.tag[1:])] + ["COMMIT-LOG.txt"]
        self.manifest = "".join(f"{'a' * 64}  {name}\n" for name in subjects).encode()
        self.release["assets"] = [
            {"name": name, "state": "uploaded", "size": 10, "digest": "sha256:" + "a" * 64} for name in subjects
        ] + [{"name": "SHA256SUMS", "state": "uploaded", "size": len(self.manifest),
              "digest": "sha256:" + hashlib.sha256(self.manifest).hexdigest()}]
        self.enterContext(patch.object(pin, "checksum_manifest", side_effect=lambda asset: self.manifest))
        self.listed = [self.tag, "v1.1.0", "v1.2.4-rc1", "v1.2.2"]
        real = pin.command

        def command(*args, **kwargs):
            if args[0] == "gh":
                if "--paginate" in args:
                    return "\n".join(self.listed)
                return json.dumps(self.release)
            return real(*args, **kwargs)

        self.mock = patch.object(pin, "command", side_effect=command)
        self.mock.start()
        self.addCleanup(self.mock.stop)
        self.messages = io.StringIO()
        self.enterContext(contextlib.redirect_stdout(self.messages))

    @staticmethod
    def git(*args):
        return subprocess.check_output(["git", *args], text=True, stderr=subprocess.PIPE).strip()

    def staged(self):
        self.branch = pin.branch_for(self.tag, self.src)
        pin.reconcile("prepare", "main")
        return self.git("rev-parse", "HEAD")

    def test_verified_annotation_replay_and_exact_formula_only_promotion(self):
        candidate = self.staged()
        self.assertEqual(self.git("ls-remote", "origin", "refs/heads/main").split()[0], self.src)
        self.assertEqual(self.staged(), candidate, "retry must reuse the identical candidate")
        pin.reconcile("promote", "main", self.src, candidate, self.tag)
        self.assertEqual(self.git("ls-remote", "origin", "refs/heads/main").split()[0], candidate)
        self.assertFalse(self.git("ls-remote", "origin", "refs/heads/" + self.branch))
        self.assertEqual(self.git("diff", "--name-only", self.src, candidate), pin.FORMULA)
        self.assertEqual(pin.formula_at(candidate), pin.with_pin(self.original, self.tag, self.src))
        pin.reconcile("prepare", "main")
        self.assertIn("already pins", self.messages.getvalue())

    def test_moved_tag_or_published_commit_or_version_refused(self):
        candidate = self.staged()
        self.git("tag", "-fa", self.tag, "-m", "moved", candidate)
        self.git("push", "--force", "origin", "refs/tags/" + self.tag)
        with self.assertRaisesRegex(pin.PinError, "does not resolve"):
            pin.reconcile("promote", "main", self.src, candidate, self.tag)
        self.release["target_commitish"] = candidate
        with self.assertRaisesRegex(pin.PinError, "one exact source"):
            pin.reconcile("prepare", "main")
        self.release["target_commitish"] = self.src
        for key in ("draft", "prerelease"):
            with patch.dict(self.release, {key: True}):
                with self.assertRaises(pin.PinError):
                    pin.latest()

    def test_independent_change_or_changed_candidate_is_never_promoted(self):
        candidate = self.staged()
        Path("unrelated").write_text("independent\n")
        self.git("add", "unrelated")
        self.git("commit", "-qm", "Other change")
        moved = self.git("rev-parse", "HEAD")
        self.git("push", "origin", "HEAD:refs/heads/" + self.branch)
        with self.assertRaisesRegex(pin.PinError, "was moved"):
            pin.reconcile("promote", "main", self.src, candidate, self.tag)
        with self.assertRaisesRegex(pin.PinError, "formula-only"):
            pin.reconcile("promote", "main", self.src, moved, self.tag)
        with self.assertRaisesRegex(pin.PinError, "left untouched"):
            self.staged()
        self.git("checkout", "main")
        self.git("commit", "--allow-empty", "-qm", "Main moved")
        self.git("push", "origin", "main")
        latest_main = self.git("rev-parse", "HEAD")
        pin.reconcile("promote", "main", self.src, candidate, self.tag)
        self.assertEqual(self.git("ls-remote", "origin", "refs/heads/main").split()[0], latest_main)
        self.assertIn("Default branch advanced", self.messages.getvalue())

    def test_later_release_and_tampered_formula_block_earlier_promotion(self):
        candidate = self.staged()
        pin.reconcile("promote", "main", self.src, candidate, "v1.2.2")
        self.assertIn("newer release", self.messages.getvalue())
        self.assertEqual(self.git("ls-remote", "origin", "refs/heads/main").split()[0], self.src)
        self.git("checkout", "--detach", self.src)
        self.formula.write_text(pin.with_pin(self.original, self.tag, self.src) + "# unrelated change\n")
        self.git("commit", "-qam", "Other formula change")
        with self.assertRaisesRegex(pin.PinError, "formula-only"):
            pin.check_candidate(self.src, self.git("rev-parse", "HEAD"),
                                pin.with_pin(self.original, self.tag, self.src))

    def test_incomplete_or_corrupt_release_is_never_pinned(self):
        with patch.dict(self.release, {"assets": []}):
            with self.assertRaisesRegex(pin.PinError, "missing"):
                pin.reconcile("prepare", "main")
        with patch.dict(self.release["assets"][0], {"digest": "sha256:" + "b" * 64}):
            with self.assertRaisesRegex(pin.PinError, "differs"):
                pin.reconcile("prepare", "main")
        self.manifest += b" "
        with self.assertRaisesRegex(pin.PinError, "checksum manifest digest"):
            pin.reconcile("prepare", "main")

    def test_promote_cannot_recreate_a_deleted_default_branch(self):
        candidate = self.staged()
        trusted = pin.command

        def race(*args, **kwargs):
            if args[:2] == ("git", "push"):
                self.git("--git-dir", str(self.origin), "update-ref", "-d", "refs/heads/main")
            return trusted(*args, **kwargs)

        with patch.object(pin, "command", side_effect=race):
            with self.assertRaises(pin.PinError):
                pin.reconcile("promote", "main", self.src, candidate, self.tag)
        self.assertEqual(self.git("ls-remote", "origin", "refs/heads/main"), "")
        self.assertIn("Review the tested formula", self.messages.getvalue())

    def test_no_downgrade_or_digest_replacement(self):
        with self.assertRaisesRegex(pin.PinError, "newer"):
            pin.with_pin(self.original, "v1.1.0", self.src)
        with self.assertRaisesRegex(pin.PinError, "another commit"):
            pin.with_pin(self.original, "v1.2.0", self.src)
        with self.assertRaisesRegex(pin.PinError, "one official"):
            pin.with_pin(self.original.replace("fantuz/recordbox", "other/project"), self.tag, self.src)
        with self.assertRaisesRegex(pin.PinError, "one official"):
            pin.with_pin(self.original * 2, self.tag, self.src)


if __name__ == "__main__":
    unittest.main()
