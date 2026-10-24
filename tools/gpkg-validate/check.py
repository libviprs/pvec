#!/usr/bin/env python3
"""Checks the sample GeoPackages with the GeoPackage validator and GDAL, inside the container
tools/gpkg-validate.sh starts (no network, read-only root, the samples mounted read-only).

    check.py DIR

Prints the versions it used and one line per check; exits 1 if any check failed."""
import os
import re
import sqlite3
import subprocess
import sys

DIR = sys.argv[1]
VALIDATE = "/usr/lib/python3/dist-packages/osgeo_utils/samples/validate_gpkg.py"
TABLE = "pvec_features"
RTREE = "rtree_pvec_features_geom"
failed = 0


def check(name, ok, detail=""):
    global failed
    print(("PASS " if ok else "FAIL ") + name + ((": " + detail) if detail and not ok else ""))
    if not ok:
        failed += 1


def run(*args):
    return subprocess.run(args, capture_output=True, text=True)


def ledger_counts(path):
    counts = {}
    for line in open(path):
        if line.startswith("fidelity "):
            counts[line.split()[1]] = counts.get(line.split()[1], 0) + 1
    return counts


print("gdal:", run("gdalinfo", "--version").stdout.strip())
print("sqlite3 module:", sqlite3.sqlite_version, "python:", sys.version.split()[0])
for name in sorted(f for f in os.listdir(DIR) if f.endswith(".gpkg")):
    base = name[:-5]
    path = os.path.join(DIR, name)
    print("==", name, os.path.getsize(path), "bytes")
    # open read-only, immutable: the mount is read-only and nothing may be written
    db = sqlite3.connect("file:%s?mode=ro&immutable=1" % path, uri=True)
    check(base + ": integrity_check", db.execute("PRAGMA integrity_check").fetchall() == [("ok",)])
    check(base + ": foreign_key_check", db.execute("PRAGMA foreign_key_check").fetchall() == [])
    check(base + ": application_id GPKG", db.execute("PRAGMA application_id").fetchone()[0] == 0x47504B47)
    check(base + ": user_version 10400", db.execute("PRAGMA user_version").fetchone()[0] == 10400)
    try:
        r = db.execute("SELECT rtreecheck('%s')" % RTREE).fetchone()[0]
        check(base + ": rtreecheck", r == "ok", str(r))
    except sqlite3.Error as e:
        check(base + ": rtreecheck", False, str(e))
    rows = db.execute("SELECT count(*) FROM %s" % TABLE).fetchone()[0]
    indexed = db.execute("SELECT count(*) FROM %s" % RTREE).fetchone()[0]
    check(base + ": every feature is in the R-tree (%d)" % rows, rows == indexed, "%d vs %d" % (rows, indexed))
    v = run(sys.executable, VALIDATE, path)
    out = (v.stdout + v.stderr).strip()
    check(base + ": validate_gpkg.py", v.returncode == 0 and "ERROR" not in out and "Error" not in out, out[-800:])
    info = run("ogrinfo", "-al", "-so", path)
    m = re.search(r"Feature Count: (\d+)", info.stdout)
    check(base + ": ogrinfo opens it and counts %d features" % rows, info.returncode == 0 and m and int(m.group(1)) == rows, info.stdout[-400:] + info.stderr[-400:])
    if rows:
        # the spatial filter goes through the R-tree; the whole extent must return every feature
        ext = re.search(r"Extent: \(([-\d.e+]+), ([-\d.e+]+)\) - \(([-\d.e+]+), ([-\d.e+]+)\)", info.stdout)
        spat = run("ogrinfo", "-al", "-so", "-spat", *[str(float(x)) for x in (ext.group(1), ext.group(2), ext.group(3), ext.group(4))], path) if ext else None
        m2 = re.search(r"Feature Count: (\d+)", spat.stdout) if spat else None
        check(base + ": spatial filter over the extent", bool(m2) and int(m2.group(1)) == rows, (spat.stdout[-300:] if spat else "no extent"))
        wkt = run("ogr2ogr", "-f", "CSV", "/dev/stdout", path, TABLE, "-lco", "GEOMETRY=AS_WKT")
        check(base + ": ogr2ogr reads every geometry", wkt.returncode == 0 and wkt.stdout.count("\n") >= rows, wkt.stderr[-300:])
        ledger = ledger_counts(os.path.join(DIR, base + ".ledger"))
        if "CIRCULARSTRING" in wkt.stdout or "COMPOUNDCURVE" in wkt.stdout:
            print("     GDAL parsed:", sorted(set(re.findall(r"(CIRCULARSTRING|COMPOUNDCURVE|LINESTRING)", wkt.stdout))))
    if base == "mixed":
        # the arcs, measured by GDAL from the points the file holds, against the source circles:
        # entity 3 is r 5 over 2.2 rad, entity 4 is r 5 over 5.0 rad, entity 5 a whole circle r 4 at (30, 0)
        from osgeo import ogr
        ds = ogr.Open(path)
        layer = ds.GetLayerByName(TABLE)
        want = {3: (11.0, None), 4: (25.0, None), 5: (8 * 3.141592653589793, (26.0, 34.0, -4.0, 4.0))}
        got = {}
        for f in layer:
            if f.GetField("entity_id") in want:
                g = f.GetGeometryRef()
                got[f.GetField("entity_id")] = (g.GetGeometryName(), g.Length(), g.GetEnvelope())
        for eid, (length, env) in want.items():
            name, glen, genv = got[eid]
            check("mixed: GDAL reads entity %d as a CIRCULARSTRING" % eid, name == "CIRCULARSTRING", name)
            check("mixed: entity %d arc length %.9f vs %.9f" % (eid, glen, length), abs(glen - length) < 1e-9 * length, repr(glen))
            if env:
                check("mixed: entity %d envelope" % eid, all(abs(a - b) < 1e-9 for a, b in zip(genv, env)), repr(genv))
    db.close()
print("FAILED %d" % failed if failed else "ALL CHECKS PASSED")
sys.exit(1 if failed else 0)
