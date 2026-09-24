# Test lab TLS material — for tests only

Certificate and key of the Samba test domain controller (`dc`, `dc.remotehub.test`), signed by a throwaway
CA whose private key was deleted right after signing. Valid until 2046.

These files are public on purpose: they protect nothing but a test domain inside Docker. Never use them
anywhere else. Tests trust `ca.crt` to verify LDAPS like a real deployment would.
