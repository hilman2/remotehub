# SSH target of the test lab (see README.md). Run with the host name
# `ssh-target`.
FROM debian:trixie-slim
RUN apt-get update \
  && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends openssh-server \
  && rm -rf /var/lib/apt/lists/* \
  && mkdir -p /run/sshd \
  && ssh-keygen -A

RUN useradd --create-home --shell /bin/bash tester \
  && echo 'tester:Tester-Passw0rd!' | chpasswd \
  && mkdir -p /home/tester/.ssh
COPY tester_ed25519.pub tester_ed25519_passphrase.pub /tmp/keys/
RUN cat /tmp/keys/*.pub >/home/tester/.ssh/authorized_keys \
  && rm -rf /tmp/keys \
  && chown -R tester:tester /home/tester/.ssh \
  && chmod 700 /home/tester/.ssh \
  && chmod 600 /home/tester/.ssh/authorized_keys
COPY user_ca.pub /etc/ssh/remotehub-testlab-user-ca.pub
COPY sshd.conf /etc/ssh/sshd_config.d/remotehub-testlab.conf

EXPOSE 22
CMD ["/usr/sbin/sshd", "-D", "-e"]
