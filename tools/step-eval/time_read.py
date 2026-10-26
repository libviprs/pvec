"""Times the independent evaluator's read of a Part 21 file (pvec#61, P8.2).

Runs in the tools/step-eval container, which has no network and a read-only root, on a file mounted read-only.
It does what evaluate.py does up to the point where OCCT holds the model, and times the two parts apart:

    lift_ms      the textual 2D to 3D lift that the evaluator needs because OCCT imports only 3D shapes;
                 an artefact of the P6.3 harness, not a cost of reading Part 21
    read_ms      Part.read(), OCCT's parse of the file into a shape, which is the cold open of this arm
    edges        the number of edges OCCT found, which must equal the entities written
    peak_rss_kib the process's peak resident set

Output is `key=value` lines. The lift's output goes to /tmp, which is a tmpfs in the run.
"""
import os
import resource
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import evaluate  # noqa: E402  (the evaluator's own lift and Part import)

Part = evaluate.Part


def main(path):
    t0 = time.monotonic()
    with open(path) as fh:
        text = fh.read()
    lifted_text = evaluate.lift(text)
    lifted = "/tmp/lifted.stp"
    with open(lifted, "w") as fh:
        fh.write(lifted_text)
    del text, lifted_text
    t1 = time.monotonic()
    shape = Part.read(lifted)
    t2 = time.monotonic()
    edges = len(shape.Edges)
    print("lift_ms=%.3f" % ((t1 - t0) * 1000))
    print("read_ms=%.3f" % ((t2 - t1) * 1000))
    print("edges=%d" % edges)
    print("peak_rss_kib=%d" % resource.getrusage(resource.RUSAGE_SELF).ru_maxrss)


if __name__ == "__main__":
    main(sys.argv[1])
