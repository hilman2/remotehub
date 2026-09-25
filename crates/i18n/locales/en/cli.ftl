# Output of the admin command line (`remotehub …`).

break-glass-ready = Break-glass account { $username } is ready. This is shown only once; keep it offline, e.g. in a safe.
break-glass-password-label = Password:
break-glass-totp-secret-label = TOTP secret:
break-glass-totp-uri-label = TOTP URI:
break-glass-sign-in = Sign in at { $url }
break-glass-deleted = Break-glass account { $username } deleted.

account-invited = Account { $email } is created. With the link and the code, its owner sets a password and a second factor until { $expires }.
account-code-label = Code:
account-link-label = Link:

audit-intact = audit log intact: { $entries ->
    [one] 1 entry
   *[other] { $entries } entries
}
audit-broken = audit log BROKEN from entry { $first } on ({ $entries ->
    [one] 1 entry
   *[other] { $entries } entries
})
