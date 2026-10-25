"""Reads Part 21 files with a third-party parser (steputils) and prints what the parser found.

Run inside the hermetic container by tests/step_19_reader.rs. For each file it prints one `OK` or `FAIL`
line, and for an accepted file one `ENTITY e<n> <canonical form>` line per exported entity. The canonical
form is built only from what the parser returned: parameters are printed as the IEEE-754 bit pattern so
the Rust side can compare them bit for bit, and references are followed through the parser's own
instance table.
"""
import re
import struct
import sys

from steputils import p21


def bits(x):
    return "%016x" % struct.unpack("<Q", struct.pack("<d", float(x)))[0]


def reader_version():
    try:
        from importlib.metadata import version

        return version("steputils")
    except Exception:  # the wheel is imported from a zip, metadata may be unreachable
        return getattr(__import__("steputils"), "__version__", "unknown")


class Model:
    def __init__(self, data):
        self.inst = data.instances

    def ent(self, ref):
        key = str(ref)
        inst = self.inst[key]
        if hasattr(inst, "entities"):
            return {e.name: e.params for e in inst.entities}
        return {inst.entity.name: inst.entity.params}

    def one(self, ref):
        (name, params), = self.ent(ref).items()
        return name, params


def enum(x):
    return str(x).strip(".")


def num(x):
    # PARAMETER_VALUE(t) arrives as a typed parameter; a plain real arrives as a float
    if hasattr(x, "params"):
        return num(x.params if not isinstance(x.params, (tuple, list)) else x.params[0])
    return float(x)


def point(m, ref):
    name, p = m.one(ref)
    assert name == "CARTESIAN_POINT", name
    assert len(p[1]) == 2, "a 2D point has two coordinates"
    return "p(%s,%s)" % (bits(p[1][0]), bits(p[1][1]))


def direction(m, ref):
    name, p = m.one(ref)
    assert name == "DIRECTION", name
    return "d(%s,%s)" % (bits(p[1][0]), bits(p[1][1]))


def placement(m, ref):
    name, p = m.one(ref)
    assert name == "AXIS2_PLACEMENT_2D", name
    return "a(%s,%s)" % (point(m, p[1]), direction(m, p[2]))


def knots(mults, values):
    return "%s|%s" % (",".join(str(int(k)) for k in mults), ",".join(bits(v) for v in values))


def canon(m, ref):
    ent = m.ent(ref)
    if "RATIONAL_B_SPLINE_CURVE" in ent:
        bs, wk, rat = ent["B_SPLINE_CURVE"], ent["B_SPLINE_CURVE_WITH_KNOTS"], ent["RATIONAL_B_SPLINE_CURVE"]
        cps = ";".join(point(m, r) for r in bs[1])
        return "rbs(%d|%s|%s|%s|%s)" % (
            bs[0], enum(bs[3]), cps, knots(wk[0], wk[1]), ",".join(bits(w) for w in rat[0]))
    (name, p), = ent.items()
    if name == "LINE":
        vname, v = m.one(p[2])
        assert vname == "VECTOR" and float(v[2]) == 1.0
        return "line(%s,%s)" % (point(m, p[1]), direction(m, v[1]))
    if name == "CIRCLE":
        return "circle(%s,%s)" % (placement(m, p[1]), bits(p[2]))
    if name == "ELLIPSE":
        return "ellipse(%s,%s,%s)" % (placement(m, p[1]), bits(p[2]), bits(p[3]))
    if name == "POLYLINE":
        return "poly(%s)" % ";".join(point(m, r) for r in p[1])
    if name == "B_SPLINE_CURVE_WITH_KNOTS":
        cps = ";".join(point(m, r) for r in p[2])
        return "bs(%d|%s|%s|%s)" % (p[1], enum(p[4]), cps, knots(p[6], p[7]))
    if name == "TRIMMED_CURVE":
        assert enum(p[5]) == "PARAMETER"
        return "trim(%s,%s,%s,%s)" % (canon(m, p[1]), bits(num(p[2][0])), bits(num(p[3][0])), enum(p[4]))
    if name == "COMPOSITE_CURVE":
        parts = []
        for r in p[1]:
            sname, s = m.one(r)
            assert sname == "COMPOSITE_CURVE_SEGMENT" and enum(s[0]) == "CONTINUOUS"
            parts.append(canon(m, s[2]))
        return "comp(%s)" % ";".join(parts)
    raise AssertionError("no reader for " + name)


def names(m):
    out = []
    for key, inst in m.inst.items():
        parts = inst.entities if hasattr(inst, "entities") else [inst.entity]
        for e in parts:
            if e.params and isinstance(e.params[0], str) and re.fullmatch(r"e\d+", e.params[0]):
                out.append((e.params[0], key))
    return out


def main(paths):
    print("READER steputils %s, python %s" % (reader_version(), sys.version.split()[0]))
    for path in paths:
        try:
            f = p21.readfile(path)
            schema = str(f.header.get("FILE_SCHEMA"))
            assert "AP242" in schema, "schema " + schema
            m = Model(f.data[0])
            lines = ["ENTITY %s %s" % (n, canon(m, k)) for n, k in names(m)]
            print("OK %s %d entities" % (path, len(lines)))
            for l in lines:
                print(l)
        except Exception as e:  # any refusal is reported, never raised
            print("FAIL %s %s: %s" % (path, type(e).__name__, str(e).splitlines()[0] if str(e) else ""))


main(sys.argv[1:])
