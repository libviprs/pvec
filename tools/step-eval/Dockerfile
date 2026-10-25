# The independent STEP evaluator for pvec#20 (P6.3): OpenCASCADE (OCCT 7.6) reached through the
# FreeCAD Python module of Debian bookworm's own freecad-python3 package. It shares no source with
# pvec-interchange/src/step/writer.rs. FreeCAD is LGPL-2+ and OCCT is LGPL-2.1 with the OCCT exception;
# both run as a separate process in this container and are never linked into pvec, and the licence
# texts stay in /usr/share/doc/<package>/copyright inside the image.
#
# Build and publish with tools/step-eval/publish.sh. The base is pinned by digest (debian:bookworm-slim index).
FROM debian:bookworm-slim@sha256:7ff6ba31527b9e336162fa061746c366c4b2032f4d2cff2078bea43f78efe3f8
LABEL org.opencontainers.image.source="https://github.com/libviprs/pvec" \
      org.opencontainers.image.description="pvec STEP evaluator: OCCT through Debian freecad-python3" \
      org.opencontainers.image.licenses="LGPL-2.1-only WITH OCCT-exception-1.0 AND LGPL-2.0-or-later"
RUN apt-get update \
 && apt-get install -y --no-install-recommends freecad-python3 python3 \
 && rm -rf /var/lib/apt/lists/*
ENV PYTHONDONTWRITEBYTECODE=1 HOME=/tmp
