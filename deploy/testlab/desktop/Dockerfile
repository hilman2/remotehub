# Desktop target of the test lab (see README.md): RDP through xrdp, VNC through
# TigerVNC. Run with the host name `desktop-target`.
FROM debian:trixie-slim
RUN apt-get update \
  && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
       tigervnc-standalone-server tigervnc-tools x11-xserver-utils xrdp xterm \
  && rm -rf /var/lib/apt/lists/*

RUN useradd --create-home --shell /bin/bash tester \
  && echo 'tester:Tester-Passw0rd!' | chpasswd \
  && mkdir -p /etc/vnc \
  && echo 'Vnc-Pw1!' | vncpasswd -f >/etc/vnc/passwd \
  && chown tester /etc/vnc/passwd \
  && chmod 600 /etc/vnc/passwd
COPY --chmod=755 xsession /home/tester/.xsession
COPY xrdp.ini /etc/xrdp/xrdp.ini
# A fresh session for every sign-in, gone when it disconnects: tests always
# see the same desktop. No drive redirection (no FUSE in a container).
RUN sed -i \
      -e 's/^Policy=.*/Policy=Separate/' \
      -e 's/^KillDisconnected=.*/KillDisconnected=true/' \
      -e 's/^DisconnectedTimeLimit=.*/DisconnectedTimeLimit=0/' \
      -e 's/^#EnableFuseMount=false/EnableFuseMount=false/' \
      /etc/xrdp/sesman.ini \
  && grep -q '^Policy=Separate' /etc/xrdp/sesman.ini \
  && grep -q '^KillDisconnected=true' /etc/xrdp/sesman.ini \
  && grep -q '^EnableFuseMount=false' /etc/xrdp/sesman.ini
COPY tls/ /etc/xrdp/tls/
RUN chown -R root:xrdp /etc/xrdp/tls && chmod 640 /etc/xrdp/tls/key.pem
COPY --chmod=755 start.sh /usr/local/bin/start-desktop

EXPOSE 3389 5900
CMD ["/usr/local/bin/start-desktop"]
