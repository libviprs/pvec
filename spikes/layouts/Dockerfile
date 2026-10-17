# Image used by tools/layouts.sh: the gate image plus flatc (to regenerate src/gen) and binutils.
# Build on MARS:  docker build --platform linux/amd64 -t pvec-layouts:1.99.1 - < spikes/layouts/Dockerfile
FROM pvec-rust:1.99.1
RUN apt-get update -qq && apt-get install -y -qq unzip binutils >/dev/null \
 && cd /tmp && curl -fsSL -o f.zip https://github.com/google/flatbuffers/releases/download/v25.12.19/Linux.flatc.binary.clang++-18.zip \
 && unzip -q f.zip && mv flatc /usr/local/bin/ && rm f.zip && flatc --version
