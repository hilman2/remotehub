# guacd for remotehub (ADR 0003): guacamole-server from the Apache source
# release, built against FreeRDP 3 from Debian — the official image still links
# FreeRDP 2, which is end of life. Only the RDP and VNC plugins: SSH is
# remotehub's own, telnet and Kubernetes are not offered.
#
# guacd listens on 4822 without authentication, so it must only be reachable
# from remotehub on an internal network: never publish its port.

ARG DEBIAN=debian:trixie-slim

FROM ${DEBIAN} AS build
ARG GUACAMOLE_VERSION=1.6.0
ARG GUACAMOLE_SHA256=8bc45675da96d7b6f39728160181e3d4ff3c08f460f6d26de5805b642bf13f2b
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      build-essential ca-certificates curl pkg-config \
      freerdp3-dev libcairo2-dev libjpeg62-turbo-dev libpng-dev libpulse-dev \
      libssl-dev libvncserver-dev libvorbis-dev libwebp-dev uuid-dev \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /build
RUN curl -fsSLo server.tar.gz \
      "https://archive.apache.org/dist/guacamole/${GUACAMOLE_VERSION}/source/guacamole-server-${GUACAMOLE_VERSION}.tar.gz" \
 && echo "${GUACAMOLE_SHA256}  server.tar.gz" | sha256sum -c - \
 && tar -xzf server.tar.gz --strip-components=1 \
# guacamole-server builds with -Werror; FreeRDP 3.15 deprecated calls that
# 1.6.0 still uses (e.g. codecs_free), which must stay warnings.
 && ./configure --prefix=/opt/guacamole \
      --with-freerdp-plugin-dir=/opt/guacamole/freerdp3 \
      --without-libssh2 --without-telnet --disable-kubernetes \
      --disable-guacenc --disable-guaclog \
      CFLAGS="-O2 -Wno-error=deprecated-declarations" \
 && make -j"$(nproc)" \
 && make install

FROM ${DEBIAN}
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates libcairo2 libfreerdp-client3-3 libfreerdp3-3 libjpeg62-turbo \
      libpng16-16t64 libpulse0 libssl3t64 libuuid1 libvncclient1 libvorbisenc2 \
      libwebp7 libwinpr3-3 \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 1000 --create-home --home-dir /home/guacd --shell /usr/sbin/nologin guacd
COPY --from=build /opt/guacamole /opt/guacamole
# FreeRDP loads channel plugins (guacd's audio input and static virtual
# channels) only from its own directory, /usr/lib/<multiarch>/freerdp3.
RUN plugins="$(echo /usr/lib/*-linux-gnu)/freerdp3" \
 && mkdir -p "$plugins" \
 && cp /opt/guacamole/freerdp3/*.so "$plugins/" \
 && ldconfig /opt/guacamole/lib
# FreeRDP keeps state below the home directory: give it a tmpfs there when
# the root filesystem is read-only.
ENV HOME=/home/guacd
USER guacd
EXPOSE 4822
ENTRYPOINT ["/opt/guacamole/sbin/guacd", "-f", "-b", "0.0.0.0", "-l", "4822", "-L", "info"]
