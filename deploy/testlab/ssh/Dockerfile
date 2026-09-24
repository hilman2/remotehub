# SSH target of the test lab (see README.md). Run with the host name
# `ssh-target`.
FROM debian:trixie-slim
RUN apt-get update \
  && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends openssh-server \
  && rm -rf /var/lib/apt/lists/* \
  && mkdir -p /run/sshd \
  && ssh-keygen -A

RUN useradd --create-home --shell /bin/bash tester \
  && echo 'tester:Tester-Passw0rd!' | chpasswd
COPY --chown=tester:tester --chmod=600 tester_ed25519.pub /home/tester/.ssh/authorized_keys
RUN chmod 700 /home/tester/.ssh
COPY sshd.conf /etc/ssh/sshd_config.d/remotehub-testlab.conf

EXPOSE 22
CMD ["/usr/sbin/sshd", "-D", "-e"]
