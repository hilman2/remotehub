#!/bin/sh
# Users and groups of the test domain. Tests rely on exactly this content:
#
#   svc-remotehub  service account for lookups           Svc-Passw0rd!
#   alice          member of "RH Admins"                  Alice-Passw0rd!
#   bob            member of "Helpdesk", which is a       Bob-Passw0rd!
#                  member of "RH Operators" (nested)
#   carol          disabled                               Carol-Passw0rd!
#   dave           no groups besides Domain Users         Dave-Passw0rd!
#
# Computers with a Windows LAPS password (msLAPS-Password) that works on the
# lab target of the same name: desktop-target (RDP) and ssh-target (SSH),
# both for the account tester. nolaps has a computer account but no LAPS
# password.
set -eu

user() { # name password given surname
  samba-tool user create "$1" "$2" --given-name="$3" --surname="$4" \
    --mail-address="$1@remotehub.test"
  samba-tool user setexpiry "$1" --noexpiry
}

user svc-remotehub 'Svc-Passw0rd!' Service remotehub
user alice 'Alice-Passw0rd!' Alice Admin
user bob 'Bob-Passw0rd!' Bob Helpdesk
user carol 'Carol-Passw0rd!' Carol Disabled
user dave 'Dave-Passw0rd!' Dave Nogroups

samba-tool group add "RH Admins"
samba-tool group add "RH Operators"
samba-tool group add "Helpdesk"
samba-tool group addmembers "RH Admins" alice
samba-tool group addmembers "Helpdesk" bob
samba-tool group addmembers "RH Operators" Helpdesk

samba-tool user disable carol

# LAPS: the schema first, in two steps (see laps.ldif), then the computers.
sam=/var/lib/samba/private/sam.ldb
ldbmodify -H "$sam" --option="dsdb:schema update allowed=true" /usr/local/share/testlab/laps.ldif
ldbmodify -H "$sam" --option="dsdb:schema update allowed=true" \
  /usr/local/share/testlab/laps-computer.ldif

computer() { # name [laps-json]
  samba-tool computer create "$1"
  {
    echo "dn: CN=$1,CN=Computers,DC=remotehub,DC=test"
    echo "changetype: modify"
    echo "replace: dNSHostName"
    echo "dNSHostName: $1.remotehub.test"
    echo "-"
    if [ -n "${2:-}" ]; then
      echo "replace: msLAPS-Password"
      echo "msLAPS-Password: $2"
      echo "-"
    fi
  } | ldbmodify -H "$sam"
}

computer desktop-target '{"n":"tester","t":"1dc2f4d1e4a5b00","p":"Tester-Passw0rd!"}'
computer ssh-target '{"n":"tester","t":"1dc2f4d1e4a5b00","p":"Tester-Passw0rd!"}'
computer nolaps
