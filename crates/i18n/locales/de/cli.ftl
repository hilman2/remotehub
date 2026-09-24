# Ausgaben der Admin-Kommandozeile (`remotehub …`).

break-glass-ready = Notfallkonto { $username } ist bereit. Das wird nur dieses eine Mal angezeigt; offline aufbewahren, z. B. in einem Tresor.
break-glass-password-label = Passwort:
break-glass-totp-secret-label = TOTP-Geheimnis:
break-glass-totp-uri-label = TOTP-URI:
break-glass-sign-in = Anmelden unter { $url }
break-glass-deleted = Notfallkonto { $username } gelöscht.

audit-intact = Audit-Log intakt: { $entries ->
    [one] 1 Eintrag
   *[other] { $entries } Einträge
}
audit-broken = Audit-Log BESCHÄDIGT ab Eintrag { $first } ({ $entries ->
    [one] 1 Eintrag
   *[other] { $entries } Einträge
})
