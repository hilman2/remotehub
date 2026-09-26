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

connector-ui-partly = Teilweise offen. remotehub erreicht nur die offenen Geräte, auf ihren Ports.
connector-ui-network = Ganzes Netz
connector-ui-network-hint = Jede Adresse, die dieser Konnektor erlaubt, auf jedem Port.
connector-ui-groups = Gruppen
connector-ui-no-groups = Noch keine Gruppen. Eine Gruppe öffnet mehrere Geräte auf einmal.
connector-ui-devices = Geräte
connector-ui-no-devices = Noch keine Geräte. Trag die ein, die remotehub einzeln erreichen darf.
connector-ui-col-name = Name
connector-ui-col-address = Adresse
connector-ui-col-ports = Ports
connector-ui-col-groups = Gruppen
connector-ui-col-members = Geräte
connector-ui-col-access = Zugang
connector-ui-row-closed = geschlossen
connector-ui-row-open-until = offen bis { $until }
connector-ui-row-open = offen ohne Ende
connector-ui-row-open-via = offen über { $group }
connector-ui-hours = { $hours ->
    [one] 1 Stunde
   *[other] { $hours } Stunden
}
connector-ui-without-end = ohne Ende
connector-ui-open = Öffnen
connector-ui-close-row = Schließen
connector-ui-remove = Entfernen
connector-ui-add-device = Gerät hinzufügen
connector-ui-add-group = Gruppe hinzufügen
connector-ui-address-hint = Eine Adresse, ein Bereich wie 10.0.0.0/24 oder ein Hostname.
connector-ui-ports-hint = Ports wie 22, 3389, 8000-8100.
connector-ui-error-name = Ein Name hat 1 bis 64 Zeichen.
connector-ui-error-address = { $value } ist weder eine Adresse noch ein Bereich noch ein Hostname.
connector-ui-error-ports = { $value } ist kein Port und kein Portbereich.
connector-ui-error-no-ports = Ein Gerät braucht mindestens einen Port.
connector-ui-error-exists = { $name } gibt es schon.
connector-ui-error-unknown = { $name } gibt es nicht.

connector-ui-requests = Anfragen von remotehub
connector-ui-requests-hint = Eine Zustimmung öffnet genau die aufgeführten Adressen und Ports, für die angefragte Zeit. Namen und Begründungen stehen so da, wie remotehub sie angibt.
connector-ui-request-asks = { $requester } bittet um Zugang für { $minutes ->
    [one] 1 Minute
   *[other] { $minutes } Minuten
}.
connector-ui-request-reason = Begründung: { $reason }
connector-ui-col-target = Name in remotehub
connector-ui-col-port = Port
connector-ui-col-listed = Ihre Liste
connector-ui-listed-as = eingetragen als { $name }
connector-ui-not-listed = nicht in Ihrer Liste
connector-ui-approve = Zustimmen
connector-ui-refuse = Ablehnen
connector-ui-approved = Zugestimmte Anfragen
connector-ui-approved-for = Für { $requester }: { $reason }
connector-ui-error-request-gone = Die Anfrage wartet nicht mehr: remotehub hat sie zurückgezogen, oder jemand hat sie schon beantwortet.

connector-log-what-network = den Zugang
connector-log-what-device = das Gerät { $name }
connector-log-what-group = die Gruppe { $name }
connector-log-what-request = die Anfrage { $id }
connector-log-opened-until = { $who } hat { $what } bis { $until } geöffnet.
connector-log-opened-until-cli = Auf der Kommandozeile { $what } bis { $until } geöffnet.
connector-log-opened-permanent = { $who } hat { $what } ohne Ende geöffnet.
connector-log-opened-permanent-cli = Auf der Kommandozeile { $what } ohne Ende geöffnet.
connector-log-closed = { $who } hat { $what } geschlossen.
connector-log-closed-cli = Auf der Kommandozeile { $what } geschlossen.
connector-log-expired = Die Zeit ist abgelaufen: { $what } geschlossen.
connector-log-command-line = Die Kommandozeile
connector-log-device-added = { $who } hat das Gerät { $name } hinzugefügt: { $address }, Ports { $ports }.
connector-log-device-removed = { $who } hat das Gerät { $name } entfernt.
connector-log-group-added = { $who } hat die Gruppe { $name } hinzugefügt.
connector-log-group-removed = { $who } hat die Gruppe { $name } entfernt.
connector-log-request-received = remotehub fragt an, Anfrage { $id }: { $requester } möchte { $targets } für { $minutes ->
    [one] 1 Minute
   *[other] { $minutes } Minuten
}. Begründung: { $reason }
connector-log-request-approved = { $who } hat Anfrage { $id } von { $requester } zugestimmt: { $targets } bis { $until }.
connector-log-request-refused = { $who } hat Anfrage { $id } von { $requester } abgelehnt: { $targets }.
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

connector-cli-device-added = Gerät { $name } hinzugefügt. Es ist geschlossen.
connector-cli-device-removed = Gerät { $name } entfernt.
connector-cli-group-added = Gruppe { $name } hinzugefügt. Sie ist geschlossen.
connector-cli-group-removed = Gruppe { $name } entfernt.
connector-cli-item-closed = { $name }: geschlossen
connector-cli-item-open-until = { $name }: offen bis { $until }
connector-cli-item-open-permanent = { $name }: offen ohne Ende
connector-cli-groups-label = Gruppen:
connector-cli-devices-label = Geräte:
