#!/usr/bin/env python3
"""Packs Seed Map into a Horizon native-mod bundle (.hnmod).

    tools/package.py [--src DIR] [--out DIR] [--check]

Reads horizon-mixin.toml, packs it with every file it references (`source` of a module,
`helper` of an injection), writes horizon-native.json with the same id, version and Horizon
revision, and refuses to package when anything disagrees: a referenced file is missing, the
version is not X.Y.Z, the revision is not the one this mod targets, or a `VERSION` constant in
the sources differs from the manifest's. The zip is reproducible (fixed timestamps and order).
`--check` validates and writes nothing. It never installs anything.
"""
import argparse, json, re, sys, zipfile
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover
    sys.exit("Python 3.11+ is needed (tomllib)")

REVISION = "66f3981f522b24c250b21523a70080965b940424"
# The horizon-mixins tool revision every installed bundle (e.g. Chat Translate 0.4.1) names.
MIXINS = "0d6b103c70633d8fd9443d8d0acd3aadaacedb06"
GAME = "26.3"
ID = "seedmap"
STAMP = (2026, 11, 5, 0, 0, 0)


class PackageError(Exception):
    pass


def referenced(manifest: dict) -> list[str]:
    names = []
    for key, field in (("module", "source"), ("injection", "helper")):
        for entry in manifest.get(key, []):
            name = entry.get(field)
            if not name:
                raise PackageError(f"a [[{key}]] has no {field}")
            if "/" in name or name.startswith("."):
                raise PackageError(f"{field} {name!r} must be a plain file name")
            if name not in names:
                names.append(name)
    return names


def check(src: Path, revision: str = REVISION) -> tuple[dict, list[str]]:
    toml_path = src / "horizon-mixin.toml"
    if not toml_path.is_file():
        raise PackageError(f"{toml_path} is missing")
    manifest = tomllib.loads(toml_path.read_text())
    mod = manifest.get("mod", {})
    if mod.get("id") != ID:
        raise PackageError(f"mod id is {mod.get('id')!r}, expected {ID!r}")
    version = mod.get("version", "")
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        raise PackageError(f"version {version!r} is not X.Y.Z")
    got = manifest.get("compatibility", {}).get("horizon_revision")
    if got != revision:
        raise PackageError(f"horizon_revision is {got!r}, expected {revision}")
    files = referenced(manifest)
    missing = [n for n in files if not (src / n).is_file()]
    if missing:
        raise PackageError("referenced files missing: " + ", ".join(missing))
    for name in files:
        for found in re.findall(r'const\s+VERSION\s*:\s*&str\s*=\s*"([^"]*)"', (src / name).read_text()):
            if found != version:
                raise PackageError(f"{name} says version {found}, the manifest says {version}")
    return manifest, files


def build(src: Path, out: Path, revision: str = REVISION) -> Path:
    src, out = Path(src), Path(out)
    manifest, files = check(src, revision)
    version = manifest["mod"]["version"]
    meta = {"game": GAME, "horizon": revision, "id": ID, "mixins": MIXINS, "schema": 1, "version": version}
    out.mkdir(parents=True, exist_ok=True)
    path = out / f"{ID}-{version}.hnmod"
    entries = [("horizon-native.json", (json.dumps(meta, indent=2, sort_keys=True) + "\n").encode())]
    entries.append((f"mixins/{ID}/horizon-mixin.toml", (src / "horizon-mixin.toml").read_bytes()))
    for name in sorted(files):
        entries.append((f"mixins/{ID}/{name}", (src / name).read_bytes()))
    tmp = path.with_suffix(".tmp")
    with zipfile.ZipFile(tmp, "w", zipfile.ZIP_DEFLATED) as z:
        for name, data in entries:
            info = zipfile.ZipInfo(name, STAMP)
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o644 << 16
            z.writestr(info, data)
    tmp.replace(path)
    return path


def main() -> int:
    here = Path(__file__).resolve().parent.parent
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--src", type=Path, default=here)
    ap.add_argument("--out", type=Path, default=here / "dist")
    ap.add_argument("--revision", default=REVISION)
    ap.add_argument("--check", action="store_true", help="validate only, write nothing")
    args = ap.parse_args()
    try:
        if args.check:
            manifest, files = check(args.src, args.revision)
            print(f"ok: {ID} {manifest['mod']['version']}, {len(files)} files + manifest")
        else:
            print(build(args.src, args.out, args.revision))
    except PackageError as e:
        print(f"package: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
