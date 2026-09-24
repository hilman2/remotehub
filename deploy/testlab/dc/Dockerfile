# Samba AD domain controller of the test lab — a throwaway domain for tests.
#
# Domain REMOTEHUB.TEST (NetBIOS REMOTEHUB), base DN DC=remotehub,DC=test,
# provisioned at build time so the container starts in seconds. Users and
# groups: see users.sh. LDAPS on 636 with the certificate in tls/ (signed by
# the test CA tls/ca.crt). Run with the host name `dc`.
FROM debian:trixie-slim
RUN apt-get update \
  && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
     samba-ad-dc samba-ad-provision ldb-tools \
  && rm -rf /var/lib/apt/lists/*

# Image builds run without CAP_SYS_ADMIN, so security.* xattrs cannot be
# written; xattr_tdb keeps them in a tdb instead (as Samba's own test suite
# does).
RUN rm -f /etc/samba/smb.conf \
  && samba-tool domain provision \
       --realm=REMOTEHUB.TEST --domain=REMOTEHUB --server-role=dc \
       --dns-backend=SAMBA_INTERNAL --host-name=dc \
       --adminpass='Admin-Passw0rd!' \
       --option="vfs objects = dfs_samba4 acl_xattr xattr_tdb" \
       --option="xattr_tdb:file = /var/lib/samba/xattr.tdb" \
       --option="dns forwarder = 127.0.0.11" \
       --option="tls enabled = yes" \
       --option="tls keyfile = /etc/samba/tls/dc.key" \
       --option="tls certfile = /etc/samba/tls/dc.crt" \
       --option="tls cafile = /etc/samba/tls/ca.crt"

COPY tls/ca.crt tls/dc.crt tls/dc.key /etc/samba/tls/
RUN chmod 600 /etc/samba/tls/dc.key

COPY users.sh /usr/local/bin/users.sh
RUN sh /usr/local/bin/users.sh

EXPOSE 389 636
CMD ["samba", "--foreground", "--no-process-group", "--debug-stdout"]
