# Der Standort-Konnektor (`remotehub-connector`): Weboberfläche, Protokoll und Kommandozeile.

connector-ui-title = remotehub-Konnektor
connector-ui-sign-in = Anmelden
connector-ui-name = Name
connector-ui-password = Passwort
connector-ui-code = Code
connector-ui-code-hint = Der Code aus der Authenticator-App, falls dein Benutzer einen hat.
connector-ui-sign-in-failed = Name, Passwort oder Code stimmt nicht.
connector-ui-locked = Zu viele Fehlversuche. Versuch es in { $minutes ->
    [one] 1 Minute
   *[other] { $minutes } Minuten
} wieder.
connector-ui-sign-out = Abmelden
connector-ui-signed-in-as = Angemeldet als { $name }
connector-ui-access-for = Fernzugriff durch { $remotehub }
connector-ui-closed = Geschlossen. remotehub erreicht dieses Netz nicht.
connector-ui-open-until = Offen bis { $until }. remotehub erreicht die Geräte, die dieser Konnektor erlaubt.
connector-ui-open-permanent = Offen ohne Ende. remotehub erreicht die Geräte, die dieser Konnektor erlaubt, bis jemand den Zugang schließt.
connector-ui-changed-by = Zuletzt geändert von { $who } am { $at }.
connector-ui-changed-cli = Zuletzt auf der Kommandozeile geändert am { $at }.
connector-ui-changed-expiry = Geschlossen am { $at }, als die Zeit abgelaufen war.
connector-ui-change = Zugang ändern
connector-ui-open-for = { $hours ->
    [one] 1 Stunde öffnen
   *[other] { $hours } Stunden öffnen
}
connector-ui-until-field = Ende
connector-ui-open-until-button = Bis dahin öffnen
connector-ui-open-permanent-button = Ohne Ende öffnen
connector-ui-open-permanent-hint = Bleibt offen, bis jemand ihn schließt. Das Protokoll hält es fest.
connector-ui-close = Jetzt schließen
connector-ui-close-hint = Laufende Verbindungen enden sofort.
connector-ui-invalid-until = Wähle einen Zeitpunkt in der Zukunft.
connector-ui-connections = Laufende Verbindungen
connector-ui-no-connections = Keine Verbindungen.
connector-ui-col-device = Gerät
connector-ui-col-user = Benutzer
connector-ui-col-since = Seit
connector-ui-col-traffic = Übertragen
connector-ui-traffic = { $sent } gesendet, { $received } empfangen
connector-ui-user-hint = Benutzer stehen so da, wie remotehub sie meldet.
connector-ui-log = Protokoll
connector-ui-no-log = Noch nichts protokolliert.
connector-ui-col-time = Zeit
connector-ui-col-event = Ereignis

connector-log-opened-until = { $who } hat den Zugang bis { $until } geöffnet.
connector-log-opened-until-cli = Zugang auf der Kommandozeile bis { $until } geöffnet.
connector-log-opened-permanent = { $who } hat den Zugang ohne Ende geöffnet.
connector-log-opened-permanent-cli = Zugang auf der Kommandozeile ohne Ende geöffnet.
connector-log-closed = { $who } hat den Zugang geschlossen.
connector-log-closed-cli = Zugang auf der Kommandozeile geschlossen.
connector-log-expired = Zugang geschlossen: Die Zeit ist abgelaufen.
connector-log-connection-started = { $user } hat sich mit { $target } verbunden.
connector-log-connection-ended = { $user } hat die Verbindung mit { $target } nach { $minutes ->
    [one] 1 Minute
   *[other] { $minutes } Minuten
} beendet: { $sent } gesendet, { $received } empfangen.
connector-log-connection-refused = { $user } wollte { $target } erreichen, was dem Konnektor nicht gelang: { $reason }
connector-log-signed-in = { $user } hat sich von { $address } angemeldet.
connector-log-sign-in-failed = Fehlgeschlagene Anmeldung als { $user } von { $address }.
connector-log-locked = { $user } ist nach zu vielen Fehlversuchen für { $minutes ->
    [one] 1 Minute
   *[other] { $minutes } Minuten
} gesperrt.

connector-cli-closed = Der Zugang ist geschlossen.
connector-cli-open-until = Der Zugang ist offen bis { $until }.
connector-cli-open-permanent = Der Zugang ist offen ohne Ende.
connector-cli-user-ready = Benutzer { $name } kann sich an der Weboberfläche anmelden. Das wird nur dieses eine Mal angezeigt.
connector-cli-password-label = Passwort:
connector-cli-totp-secret-label = TOTP-Geheimnis:
connector-cli-totp-uri-label = TOTP-URI:
connector-cli-user-deleted = Benutzer { $name } gelöscht.
connector-cli-no-users = Noch keine Benutzer. Anlegen mit: remotehub-connector user add NAME
connector-cli-with-totp = mit zweitem Faktor

connector-cli-token-prompt = Füge das Token des Konnektors aus remotehub ein und drücke Enter:
connector-cli-installed = Der Windows-Dienst remotehub-connector läuft und meldet an { $url }. Der Zugang ist geschlossen. Als Nächstes einen Benutzer für die Weboberfläche anlegen mit: remotehub-connector user add NAME. Dann auf diesem Server https://localhost:8480 öffnen.
connector-cli-updated = Der Dienst läuft jetzt mit Release { $version }.
connector-cli-uninstalled = Dienst und Programm sind entfernt.
connector-cli-data-kept = Die Daten bleiben in { $path }; uninstall --purge entfernt sie.
