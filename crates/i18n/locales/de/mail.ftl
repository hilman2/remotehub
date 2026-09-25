# Mails, die remotehub verschickt (#145). Die Texte sind reiner Text;
# Leerzeilen trennen die Absätze im HTML-Teil.

mail-test-subject = remotehub-Testmail
mail-test-body = Dies ist eine Testmail von remotehub. Der Mailserver funktioniert.

mail-recovery-subject = Code für remotehub
mail-recovery-body =
    Für dieses remotehub-Konto wurde ein neues Passwort angefordert. Mit diesem Code lässt es sich setzen:

    { $code }

    Wer die Anfrage nicht selbst gestellt hat, kann diese Mail ignorieren: Das Passwort bleibt unverändert.

mail-invitation-subject = Ein Konto auf remotehub
mail-invitation-body =
    Hallo { $name },

    auf remotehub wartet ein Konto. Mit dem Link und dem Code werden ein Passwort und eine Authenticator-App eingerichtet.

    { $link }
    Code: { $code }

    Link und Code gelten bis { $expires }.

mail-sign-in-code-subject = Ein neuer Anmeldecode für remotehub
mail-sign-in-code-body =
    Hallo { $name },

    ein Administrator hat für dieses remotehub-Konto einen neuen Anmeldecode erstellt. Mit dem Link und dem Code wird die Anmeldung neu eingerichtet.

    { $link }
    Code: { $code }

    Link und Code gelten bis { $expires }.
