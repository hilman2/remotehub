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
# alice has no password and no key: she signs in only with a certificate
# from remotehub's CA (remotehub_ca, principal "alice").
RUN useradd --create-home --shell /bin/bash alice \
  && usermod --password '*' alice
COPY user_ca.pub remotehub_ca.pub /tmp/cas/
RUN cat /tmp/cas/*.pub >/etc/ssh/remotehub-testlab-user-ca.pub && rm -rf /tmp/cas
COPY sshd.conf /etc/ssh/sshd_config.d/remotehub-testlab.conf

EXPOSE 22
CMD ["/usr/sbin/sshd", "-D", "-e"]
