# Mails remotehub sends (#145). Bodies are plain text; blank lines make
# paragraphs in the HTML part.

mail-test-subject = remotehub test mail
mail-test-body = This is a test mail from remotehub. The mail server works.

mail-recovery-subject = Code for remotehub
mail-recovery-body =
    Someone asked to set a new password for this remotehub account. This code lets it be set:

    { $code }

    If that was not you, ignore this mail: the password stays as it is.

mail-invitation-subject = An account on remotehub
mail-invitation-body =
    Hello { $name },

    an account on remotehub is waiting for you. With the link and the code, you set a password and an authenticator app.

    { $link }
    Code: { $code }

    Link and code work until { $expires }.

mail-sign-in-code-subject = A new sign-in code for remotehub
mail-sign-in-code-body =
    Hello { $name },

    an administrator made a new sign-in code for your remotehub account. With the link and the code, you set up your sign-in again.

    { $link }
    Code: { $code }

    Link and code work until { $expires }.
