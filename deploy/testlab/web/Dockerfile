# Web target of the test lab (see README.md): a sign-in page over HTTPS with
# a self-signed certificate, like an appliance's. Run with the host name
# `web-target`.
FROM debian:trixie-slim
RUN apt-get update \
  && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends openssl python3 \
  && rm -rf /var/lib/apt/lists/*
# A new key with every image build: tests pin whatever the target presents.
RUN mkdir /etc/web \
  && openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -subj /CN=web-target \
       -keyout /etc/web/key.pem -out /etc/web/cert.pem \
  && chmod 644 /etc/web/key.pem
COPY server.py /usr/local/bin/web-target.py
USER nobody
EXPOSE 443
CMD ["python3", "-u", "/usr/local/bin/web-target.py"]
