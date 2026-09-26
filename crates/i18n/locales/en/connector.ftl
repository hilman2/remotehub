# The site connector (`remotehub-connector`): its web interface, its journal and its command line.

connector-ui-title = remotehub connector
connector-ui-sign-in = Sign in
connector-ui-name = Name
connector-ui-password = Password
connector-ui-code = Code
connector-ui-code-hint = The code from the authenticator app, if your user has one.
connector-ui-sign-in-failed = Name, password or code is wrong.
connector-ui-locked = Too many failed attempts. Try again in { $minutes ->
    [one] 1 minute
   *[other] { $minutes } minutes
}.
connector-ui-sign-out = Sign out
connector-ui-signed-in-as = Signed in as { $name }
connector-ui-access-for = Remote access by { $remotehub }
connector-ui-closed = Closed. remotehub cannot reach this network.
connector-ui-open-until = Open until { $until }. remotehub can reach the devices this connector allows.
connector-ui-open-permanent = Open without end. remotehub can reach the devices this connector allows until someone closes access.
connector-ui-changed-by = Last changed by { $who } at { $at }.
connector-ui-changed-cli = Last changed on the command line at { $at }.
connector-ui-changed-expiry = Closed at { $at } when the time ran out.
connector-ui-change = Change access
connector-ui-open-for = Open for { $hours ->
    [one] 1 hour
   *[other] { $hours } hours
}
connector-ui-until-field = End
connector-ui-open-until-button = Open until then
connector-ui-open-permanent-button = Open without end
connector-ui-open-permanent-hint = Stays open until someone closes it. The log marks it.
connector-ui-close = Close now
connector-ui-close-hint = Running connections end at once.
connector-ui-invalid-until = Choose a point in time in the future.
connector-ui-connections = Running connections
connector-ui-no-connections = No connections.
connector-ui-col-device = Device
connector-ui-col-user = User
connector-ui-col-since = Since
connector-ui-col-traffic = Transferred
connector-ui-traffic = { $sent } sent, { $received } received
connector-ui-user-hint = Users are named as remotehub reports them.
connector-ui-log = Log
connector-ui-no-log = Nothing logged yet.
connector-ui-col-time = Time
connector-ui-col-event = Event

connector-log-opened-until = { $who } opened access until { $until }.
connector-log-opened-until-cli = Access opened on the command line until { $until }.
connector-log-opened-permanent = { $who } opened access without end.
connector-log-opened-permanent-cli = Access opened on the command line without end.
connector-log-closed = { $who } closed access.
connector-log-closed-cli = Access closed on the command line.
connector-log-expired = Access closed: its time ran out.
connector-log-connection-started = { $user } connected to { $target }.
connector-log-connection-ended = { $user } disconnected from { $target } after { $minutes ->
    [one] 1 minute
   *[other] { $minutes } minutes
}: { $sent } sent, { $received } received.
connector-log-connection-refused = { $user } asked for { $target }, which the connector did not reach: { $reason }
connector-log-signed-in = { $user } signed in from { $address }.
connector-log-sign-in-failed = Failed sign-in as { $user } from { $address }.
connector-log-locked = { $user } is locked for { $minutes ->
    [one] 1 minute
   *[other] { $minutes } minutes
} after too many failed sign-ins.

connector-cli-closed = Access is closed.
connector-cli-open-until = Access is open until { $until }.
connector-cli-open-permanent = Access is open without end.
connector-cli-user-ready = User { $name } can sign in to the web interface. This is shown only once.
connector-cli-password-label = Password:
connector-cli-totp-secret-label = TOTP secret:
connector-cli-totp-uri-label = TOTP URI:
connector-cli-user-deleted = User { $name } deleted.
connector-cli-no-users = No users yet. Create one with: remotehub-connector user add NAME
connector-cli-with-totp = with a second factor
