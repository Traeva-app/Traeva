"""Compile E's producer against the unmodified, pinned A #151 core."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

PIN = "3d1a4a98b3afaa51e66ea1a276aa83f3778f385c"
parser = argparse.ArgumentParser()
parser.add_argument("--a-checkout", type=Path, required=True)
parser.add_argument("--output", type=Path, required=True)
args = parser.parse_args()
a = args.a_checkout.resolve()
head = subprocess.check_output(["git", "-C", str(a), "rev-parse", "HEAD"], text=True).strip()
if head != PIN:
    raise SystemExit(f"Expected A helper commit {PIN}, got {head}")
if subprocess.check_output(["git", "-C", str(a), "status", "--porcelain"], text=True):
    raise SystemExit("A helper checkout must be clean")
here = Path(__file__).resolve().parent
core = here.parent.parent
out = args.output.resolve()
out.mkdir(parents=True, exist_ok=False)
manifest = (here / "Cargo.toml").read_text()
manifest = manifest.replace('name = "traeva-pptx-validation"', 'name = "traeva-pptx-a151-compat"')
manifest = manifest.replace('[[bin]]\nname = "pptx-probe"\npath = "probe.rs"\n\n', "")
manifest = manifest.replace('traeva-core = { path = "../.." }',
    'traeva-core = { path = ' + json.dumps(str(a / "crates/traeva-core")) + ' }')
(out / "Cargo.toml").write_text(manifest)
lock = (here / "Cargo.lock").read_text().replace('name = "traeva-pptx-validation"', 'name = "traeva-pptx-a151-compat"')
(out / "Cargo.lock").write_text(lock)
(out / "lib.rs").write_text(
    'pub use traeva_core::{MAX_MEMORY_TEXT_CHARS, TextExtraction, TextStatus, TextSummary};\n'
    '#[path = ' + json.dumps(str(core / "src/pptx.rs"), ensure_ascii=False) + '] pub mod pptx;\n'
    '#[cfg(test)] #[path = ' + json.dumps(str(here / "a151_compat.rs"), ensure_ascii=False) + '] mod a151_compat;\n')
reader = a / "crates/traeva-core/src/reader.rs"
record = {"a_sha": head, "a_reader_sha256": hashlib.sha256(reader.read_bytes()).hexdigest(),
          "e_source_sha256": hashlib.sha256((core / "src/pptx.rs").read_bytes()).hexdigest(),
          "method": "Direct path dependency on real A core; no copied recognizer", "output": str(out)}
record["e_inputs_sha256"] = {
    str(path.relative_to(core)): hashlib.sha256(path.read_bytes()).hexdigest()
    for path in [core / "src/pptx_projection.rs", core / "src/pptx_package.rs",
                 here / "a151_compat.rs", here / "Cargo.toml", here / "Cargo.lock"]
}
(out / "inputs.json").write_text(json.dumps(record, indent=2) + "\n")
with (out / "test.log").open("w") as log:
    result = subprocess.run(["cargo", "test", "--manifest-path", str(out / "Cargo.toml"),
        "--offline", "--locked", "a151_compat"], stdout=log, stderr=subprocess.STDOUT)
record["exit_code"] = result.returncode
(out / "result.json").write_text(json.dumps(record, indent=2) + "\n")
print(json.dumps(record))
raise SystemExit(result.returncode)
