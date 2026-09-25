# Ausgaben der Admin-Kommandozeile (`remotehub …`).

break-glass-ready = Notfallkonto { $username } ist bereit. Das wird nur dieses eine Mal angezeigt; offline aufbewahren, z. B. in einem Tresor.
break-glass-password-label = Passwort:
break-glass-totp-secret-label = TOTP-Geheimnis:
break-glass-totp-uri-label = TOTP-URI:
break-glass-sign-in = Anmelden unter { $url }
break-glass-deleted = Notfallkonto { $username } gelöscht.

account-invited = Konto { $email } ist angelegt. Mit Link und Code setzt der Besitzer bis { $expires } Passwort und zweiten Faktor.
account-code-label = Code:
account-link-label = Link:

setup-ready = Mit diesem Link wird remotehub eingerichtet. Er gilt, bis der Assistent den ersten Administrator angelegt hat; ein erneuter Aufruf dieses Befehls ersetzt ihn.
setup-complete = remotehub ist schon eingerichtet, einen Einrichtungscode gibt es nicht mehr.

audit-intact = Audit-Log intakt: { $entries ->
    [one] 1 Eintrag
   *[other] { $entries } Einträge
}
audit-broken = Audit-Log BESCHÄDIGT ab Eintrag { $first } ({ $entries ->
    [one] 1 Eintrag
   *[other] { $entries } Einträge
})
