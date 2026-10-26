"""A throttled range source for the load-shed scenario (pvec#158). It runs inside the hermetic container
and is driven through the pipes of one `docker exec -i`; there is no network. The artifact is mounted
read-only at /corpus/artifact. Requests are lines on standard input:

    L              -> OK <length of the artifact>
    R <offset> <n> -> OK <n> and then exactly n bytes, after the throttle: the latency plus n / bandwidth
                      (a range outside the artifact is ERR <reason>, with no bytes)

usage: range_server.py LATENCY_MS BYTES_PER_SEC
"""
import os
import sys
import time

latency = int(sys.argv[1]) / 1000.0
bandwidth = int(sys.argv[2])
inp, out = sys.stdin.buffer, sys.stdout.buffer
fd = os.open("/corpus/artifact", os.O_RDONLY)
size = os.fstat(fd).st_size


def say(text):
    out.write(text.encode() + b"\n")
    out.flush()


for raw in inp:
    parts = raw.split()
    if parts == [b"L"]:
        say("OK %d" % size)
    elif len(parts) == 3 and parts[0] == b"R" and parts[1].isdigit() and parts[2].isdigit():
        offset, n = int(parts[1]), int(parts[2])
        if offset + n > size:
            say("ERR range %d+%d is outside %d bytes" % (offset, n, size))
            continue
        time.sleep(latency + n / bandwidth)
        data = os.pread(fd, n, offset)
        out.write(b"OK %d\n" % len(data) + data)
        out.flush()
    else:
        say("ERR bad request")
