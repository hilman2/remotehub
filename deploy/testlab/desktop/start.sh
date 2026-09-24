#!/bin/sh
# Starts the desktop target: a VNC display of its own on 5900, then xrdp on
# 3389, which starts an Xvnc session per RDP sign-in.
set -eu

mkdir -p /tmp/.X11-unix && chmod 1777 /tmp/.X11-unix

runuser -u tester -- Xvnc :1 -rfbport 5900 -rfbauth /etc/vnc/passwd -SecurityTypes VncAuth \
  -geometry 1280x800 -depth 24 -nolisten tcp -desktop "remotehub lab VNC" &
# Wait for the display before starting its session.
for _ in $(seq 1 50); do [ -S /tmp/.X11-unix/X1 ] && break; sleep 0.1; done
runuser -u tester -- env DISPLAY=:1 LAB_PROTOCOL=VNC HOME=/home/tester /home/tester/.xsession &

xrdp-sesman --nodaemon &
exec xrdp --nodaemon
