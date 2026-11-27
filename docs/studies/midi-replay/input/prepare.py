import hashlib
import json
import subprocess
import sys
from pathlib import Path

repo = Path(sys.argv[1]).resolve()
out = Path(__file__).resolve().parent
revisions = {"fork":"9baccd5a09facd9bc4db04222a6d35aa2792c3e8","upstream":"b55b775f91083e04f8732dfa4eae6cd1293866be"}
fixture_ref = "3c80a2fd8024573a4da323b560aae4543bdc4f73:docs/studies/DDJ-GRV6-two-deck-check.csv"
def object_bytes(ref):
    return subprocess.check_output(["git", "-C", str(repo), "show", ref])
def digest(data):
    return hashlib.sha256(data).hexdigest()
mapping = object_bytes(fixture_ref)
assert digest(mapping) == "ec16e54f996c98fa0d6629ab044e4a90f1a428fa39401d5a4e626da461f683a0"
(out / "DDJ-GRV6-two-deck-check.csv").write_bytes(mapping)
manifest = {"fixture": fixture_ref, "fixture_sha256":digest(mapping), "revisions":{}}
for key, sha in revisions.items():
    d = out / key
    d.mkdir(exist_ok=True)
    files = {}
    for name in ("midi_csv.rs","midi_map.rs"):
        data = object_bytes(sha + ":app/src/" + name)
        (d / name).write_bytes(data)
        files[name] = digest(data)
    manifest["revisions"][key] = {"commit": sha, "files": files}
    (d / "main.rs").write_text('#![allow(dead_code)]\nmod midi_csv;\nmod midi_map;\n#[path="../study.rs"] mod study;\nfn main() { study::run() }\n')
for name in ("Cargo.toml","Cargo.lock","study.rs"):
    manifest[name] = digest((out / name).read_bytes())
(out / "provenance.json").write_text(json.dumps(manifest, indent=2) + "\n")
