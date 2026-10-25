"""The independent STEP evaluator for pvec#20 (P6.3): OpenCASCADE through FreeCAD's Python module.

Runs inside the tools/step-eval container, which has no network and a read-only root. It shares no
source with pvec-interchange/src/step/writer.rs.

    evaluate.py FILE                 print the structure of every edge OCCT found
    evaluate.py FILE REQUESTS_JSON   as above, plus the points and tangents OCCT evaluates at the
                                     parameters asked for: {"<edge index>": [u, ...]}

The writer's file is a 2D profile (a 2-dimensional representation context, 2-component points and
AXIS2_PLACEMENT_2D). OCCT's STEP reader only imports 3D representations that hang off a product, so the
file is first lifted, textually and only by appending a zero third coordinate, turning each
AXIS2_PLACEMENT_2D into an AXIS2_PLACEMENT_3D and adding the units and the product chain OCCT needs. No
curve parameter is touched by the lift. Everything after that, the parsing of the curves, their
parameterisation and their evaluation, is OCCT's.

Output is line based, so the Rust side needs no JSON: `EVALUATOR`, `EDGE <i> <kind> key=v,v,...` and
`SAMPLE <edge> <u> px,py,tx,ty`, with floats in repr form so they read back exactly.
"""
import json
import re
import sys

sys.path.append("/usr/lib/freecad-python3/lib")
import FreeCAD  # noqa: E402
import Part  # noqa: E402

UNITS = (
    "#900=( LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.) );\n"
    "#901=( NAMED_UNIT(*) SI_UNIT($,.STERADIAN.) SOLID_ANGLE_UNIT() );\n"
    "#902=UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.E-7),#900,'distance_accuracy_value','');\n"
)
PRODUCT = (
    "#910=APPLICATION_CONTEXT('pvec');\n"
    "#911=APPLICATION_PROTOCOL_DEFINITION('international standard','ap242_managed_model_based_3d_engineering',2011,#910);\n"
    "#912=PRODUCT_CONTEXT('',#910,'mechanical');\n"
    "#913=PRODUCT('pvec','pvec','',(#912));\n"
    "#914=PRODUCT_DEFINITION_FORMATION('','',#913);\n"
    "#915=PRODUCT_DEFINITION_CONTEXT('part definition',#910,'design');\n"
    "#916=PRODUCT_DEFINITION('design','',#914,#915);\n"
    "#917=PRODUCT_DEFINITION_SHAPE('','',#916);\n"
)


def lift(text):
    if "GEOMETRIC_REPRESENTATION_CONTEXT(2)" not in text:
        raise ValueError("not a 2D PVec export")
    text = text.replace("GEOMETRIC_REPRESENTATION_CONTEXT(2)", "GEOMETRIC_REPRESENTATION_CONTEXT(3)")
    text = text.replace(
        "GLOBAL_UNIT_ASSIGNED_CONTEXT((#1))",
        "GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT((#902)) GLOBAL_UNIT_ASSIGNED_CONTEXT((#900,#1,#901))",
    )
    text = re.sub(r"(CARTESIAN_POINT\('[^']*',\([^()]*)\)\)", r"\1,0.))", text)
    text = re.sub(r"(=DIRECTION\('[^']*',\([^()]*)\)\)", r"\1,0.))", text)
    text = re.sub(r"AXIS2_PLACEMENT_2D\(('[^']*'),(#\d+),(#\d+)\)", r"AXIS2_PLACEMENT_3D(\1,\2,$,\3)", text)
    rep = re.search(r"#(\d+)=SHAPE_REPRESENTATION\(", text)
    if rep is None:
        raise ValueError("no SHAPE_REPRESENTATION")
    tail = "#918=SHAPE_DEFINITION_REPRESENTATION(#917,#%s);\n" % rep.group(1)
    return text.replace("#1=(", UNITS + "#1=(", 1).replace("ENDSEC;\nEND-ISO", PRODUCT + tail + "ENDSEC;\nEND-ISO")


def xy(p):
    return [float(p.x), float(p.y)]


def num(v):
    return ",".join(repr(float(x)) for x in v)


def structure(i, e):
    c = e.Curve
    kind = type(c).__name__
    a = e.valueAt(e.FirstParameter)
    b = e.valueAt(e.LastParameter)
    rec = [
        ("first", [e.FirstParameter]),
        ("last", [e.LastParameter]),
        ("start", xy(a)),
        ("end", xy(b)),
        ("z", [a.z, b.z]),
        ("length", [e.Length]),
    ]
    if kind == "BSplineCurve":
        rec += [
            ("degree", [c.Degree]),
            ("poles", [q for p in c.getPoles() for q in xy(p)]),
            ("knots", c.getKnots()),
            ("mults", c.getMultiplicities()),
            ("weights", c.getWeights()),
            ("periodic", [1 if c.isPeriodic() else 0]),
        ]
    elif kind == "Circle":
        rec += [("center", xy(c.Center)), ("radius", [c.Radius]), ("xdir", xy(c.XAxis))]
    elif kind == "Ellipse":
        rec += [("center", xy(c.Center)), ("major", [c.MajorRadius]), ("minor", [c.MinorRadius]), ("xdir", xy(c.XAxis))]
    elif kind == "Line":
        rec += [("location", xy(c.Location)), ("direction", xy(c.Direction))]
    return "EDGE %d %s %s" % (i, kind, " ".join("%s=%s" % (k, num(v)) for k, v in rec))


def package_version(name):
    block = ""
    with open("/var/lib/dpkg/status") as fh:
        for line in fh.read().split("\n\n"):
            if line.startswith("Package: %s\n" % name):
                block = line
    for line in block.split("\n"):
        if line.startswith("Version: "):
            return line[9:]
    return "unknown"


def main(argv):
    with open(argv[1]) as fh:
        text = fh.read()
    lifted = "/tmp/lifted.stp"
    with open(lifted, "w") as fh:
        fh.write(lift(text))
    shape = Part.read(lifted)
    edges = shape.Edges
    ver = FreeCAD.Version()
    print("EVALUATOR freecad=%s occt=%s edges=%d" % (
        ".".join(ver[0:3]), package_version("libocct-foundation-7.6").split("+")[0], len(edges)))
    for i, e in enumerate(edges):
        print(structure(i, e))
    if len(argv) > 2:
        for k, us in json.loads(argv[2]).items():
            c = edges[int(k)].Curve
            for u in us:
                p = c.value(float(u))
                t = c.tangent(float(u))[0]
                print("SAMPLE %s %s %s" % (k, repr(float(u)), num([p.x, p.y, t.x, t.y])))


if __name__ == "__main__":
    main(sys.argv)
