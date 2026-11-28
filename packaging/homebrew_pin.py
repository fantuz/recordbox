"""Reconcile recordbox's source tap with its newest published stable release."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys

from build_archive import release_archives

REPOSITORY = "fantuz/recordbox"
FORMULA = "Formula/recordbox.rb"
SEMVER = r"v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
SHA = r"[a-f0-9]{40}"
STANZA = re.compile(
    rf'(?m)^  url "https://github\.com/fantuz/recordbox\.git",\n'
    rf'      tag: +"(?P<tag>v[0-9.]+)",\n      revision: +"(?P<sha>{SHA})"$'
)


class PinError(RuntimeError):
    pass


def command(*args, env=None):
    result = subprocess.run(args, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env)
    if result.returncode:
        raise PinError(f"{args[0]} {args[1]} failed: {result.stderr.strip()}")
    return result.stdout.rstrip("\n")


def version(tag):
    match = re.fullmatch(SEMVER, tag)
    if not match:
        raise PinError(f"Unexpected stable series tag {tag!r}")
    return tuple(map(int, match.groups()))


def checksum_manifest(asset):
    if type(asset.get("id")) is not int or not 0 < asset["size"] <= 16_384:
        raise PinError("Invalid release checksum manifest")
    return subprocess.check_output(["gh", "api", "-H", "Accept: application/octet-stream",
                                    f"repos/{REPOSITORY}/releases/assets/{asset['id']}"])


def verify_assets(release, tag):
    archives = {name for _, _, _, name in release_archives(tag[1:])}
    subjects = archives | {"COMMIT-LOG.txt"}
    assets = {a["name"]: a for a in release["assets"]}
    if (set(assets) != subjects | {"SHA256SUMS"} or len(assets) != len(release["assets"])
            or any(a["state"] != "uploaded" or a["size"] <= 0 for a in assets.values())):
        raise PinError("Published release is missing a verified package or manifest")
    manifest = assets["SHA256SUMS"]
    data = checksum_manifest(manifest)
    if len(data) != manifest["size"] or "sha256:" + hashlib.sha256(data).hexdigest() != manifest.get("digest"):
        raise PinError("Published checksum manifest digest differs")
    checksums = {}
    for line in data.decode("utf-8").splitlines():
        match = re.fullmatch(r"([a-f0-9]{64})  ([^/\\]+)", line)
        if not match or match[2] in checksums:
            raise PinError("Invalid or duplicate release checksum")
        checksums[match[2]] = "sha256:" + match[1]
    if set(checksums) != subjects or any(assets[name].get("digest") != digest for name, digest in checksums.items()):
        raise PinError("Published package set or digest differs from the checksum manifest")


def latest():
    # A delayed older publication or an operator's 'latest' designation must never
    # push the tap backwards. Paginate even when a repository has many previews.
    names = command("gh", "api", "--paginate", f"repos/{REPOSITORY}/releases?per_page=100",
                    "--jq", ".[] | select(.draft == false and .prerelease == false) | .tag_name")
    stable = [n for n in names.splitlines() if re.fullmatch(SEMVER, n)]
    if not stable:
        raise PinError("No published stable recordbox release found")
    tag = max(stable, key=version)
    release = json.loads(command("gh", "api", f"repos/{REPOSITORY}/releases/tags/{tag}"))
    source = release.get("target_commitish", "")
    marker = f"<!-- recordbox-release-source: {source} -->"
    body = release.get("body") or ""
    if (release["tag_name"] != tag or release["draft"] or release["prerelease"] or not release["published_at"]
            or not re.fullmatch(SHA, source) or body.count("recordbox-release-source:") != 1 or marker not in body):
        raise PinError("Published release does not identify one exact source commit")
    tag_ref = f"refs/tags/{tag}"
    refs = dict(line.split("\t")[::-1] for line in command("git", "ls-remote", "--tags", "origin", tag_ref, tag_ref + "^{}").splitlines())
    if tag_ref not in refs or refs.get(tag_ref + "^{}", refs[tag_ref]) != source:
        raise PinError(f"Published tag {tag} does not resolve to its recorded commit")
    verify_assets(release, tag)
    return tag, source


def with_pin(formula, tag, source):
    matches = list(STANZA.finditer(formula))
    if len(matches) != 1:
        raise PinError(f"Expected one official, SHA-pinned source URL in {FORMULA}")
    match = matches[0]
    old_tag = match["tag"]
    if version(old_tag) > version(tag):
        raise PinError(f"Existing {old_tag} formula is newer than public release {tag}")
    if old_tag == tag and match["sha"] != source:
        raise PinError(f"Existing {tag} formula identifies another commit")
    return (formula[:match.start("tag")] + tag + formula[match.end("tag"):match.start("sha")]
            + source + formula[match.end("sha"):])


def summary(message):
    print(message, flush=True)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as stream:
            print(message, file=stream)


def output(**values):
    if os.environ.get("GITHUB_OUTPUT"):
        with open(os.environ["GITHUB_OUTPUT"], "a") as stream:
            for key, value in values.items():
                print(f"{key}={value}", file=stream)


def branch_for(tag, base):
    return f"automation/homebrew-{tag}-{base}"


def formula_at(commit):
    # git show emits the exact blob, including its trailing newline.
    return subprocess.check_output(["git", "show", f"{commit}:{FORMULA}"]).decode("utf-8")


def candidate_commit(base, tag, target):
    command("git", "checkout", "--detach", base)
    Path(FORMULA).write_bytes(target.encode())
    command("git", "add", "--", FORMULA)
    date = command("git", "show", "-s", "--format=%cI", base)
    env = {**os.environ, "GIT_AUTHOR_DATE": date, "GIT_COMMITTER_DATE": date}
    command("git", "-c", "user.name=github-actions[bot]", "-c",
            "user.email=41898282+github-actions[bot]@users.noreply.github.com",
            "commit", "--only", "-m", f"Pin Homebrew recordbox to {tag}", "--", FORMULA, env=env)
    return command("git", "rev-parse", "HEAD")


def check_candidate(base, commit, target):
    if (command("git", "show", "-s", "--format=%P", commit) != base
        or command("git", "diff", "--no-renames", "--name-only", base, commit) != FORMULA
        or command("git", "ls-tree", commit, "--", FORMULA).split()[:2] != ["100644", "blob"]
        or formula_at(commit) != target):
        raise PinError("Test candidate must be a single formula-only commit with the expected contents")


def reconcile(mode, default, base=None, tested=None, tested_tag=None):
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._/-]*", default):
        raise PinError("Invalid default branch")
    command("git", "check-ref-format", f"refs/heads/{default}")
    command("git", "fetch", "--no-tags", "origin", f"refs/heads/{default}")
    current = command("git", "rev-parse", "FETCH_HEAD")
    tag, source = latest()
    command("git", "merge-base", "--is-ancestor", source, current)
    if command("git", "show", f"{source}:VERSION") != tag[1:]:
        raise PinError(f"{tag} source VERSION differs")
    previous = formula_at(current)
    target = with_pin(previous, tag, source)
    if target == previous:
        summary(f"Homebrew already pins [{tag}](https://github.com/{REPOSITORY}/releases/tag/{tag}) to `{source}`.")
        return
    branch = branch_for(tag, current)
    ref = f"refs/heads/{branch}"
    compare = f"https://github.com/{REPOSITORY}/compare/{default}...{branch}?expand=1"
    if mode == "prepare":
        commit = candidate_commit(current, tag, target)
        check_candidate(current, commit, target)
        existing = command("git", "ls-remote", "--refs", "origin", ref)
        if existing and existing.split()[0] != commit:
            raise PinError(f"Existing candidate {branch} has different contents; left untouched")
        if not existing:
            command("git", "push", f"--force-with-lease={ref}:", "origin", f"{commit}:{ref}")
        output(base=current, commit=commit, tag=tag)
        summary(f"Testing [{tag} tap change]({compare}) at `{commit}` on both Mac architectures.")
        return
    if current != base:
        summary("Default branch advanced during native validation. Its successful build will reconcile the current release.")
        return
    if tested_tag != tag:
        summary("A newer release was published during native validation. Its publication will reconcile the tap.")
        return
    if not tested or not re.fullmatch(SHA, tested):
        raise PinError("Missing tested source revision")
    command("git", "fetch", "--no-tags", "origin", ref)
    if command("git", "rev-parse", "FETCH_HEAD") != tested:
        raise PinError("Tested candidate branch was moved")
    check_candidate(current, tested, target)
    try:
        # check_candidate proves this is exactly one fast-forward. An explicit lease
        # additionally rejects a concurrent reset to an older ancestor or an absent ref.
        destination = f"refs/heads/{default}"
        command("git", "push", f"--force-with-lease={destination}:{current}", "origin", f"{tested}:{destination}")
    except PinError:
        summary(f"Promotion was declined. [Review the tested formula in a pull request]({compare}).")
        raise
    summary(f"Published {tag} Homebrew update `{tested}` to `{default}` after both native installations passed.")
    try:
        command("git", "push", f"--force-with-lease={ref}:{tested}", "origin", f":{ref}")
    except PinError:
        summary(f"The candidate branch `{branch}` remains available.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["prepare", "promote"])
    args = parser.parse_args()
    try:
        reconcile(args.mode, os.environ["PIN_DEFAULT_BRANCH"], os.getenv("PIN_BASE"),
                  os.getenv("PIN_TESTED_COMMIT"), os.getenv("PIN_TESTED_TAG"))
    except (PinError, subprocess.CalledProcessError, OSError, ValueError) as exc:
        sys.exit(str(exc))
