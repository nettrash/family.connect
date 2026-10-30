# A coturn for the harness's TURN case, from the same pinned Ubuntu 26.04 snapshot as fc-linux-dev
# (its apt is already pointed at the snapshot).
FROM fc-linux-dev
RUN apt-get update \
 && apt-get install -y --no-install-recommends coturn \
 && rm -rf /var/lib/apt/lists/*
