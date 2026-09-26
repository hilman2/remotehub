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

connector-ui-partly = Partly open. remotehub reaches only the open devices, on their ports.
connector-ui-network = Whole network
connector-ui-network-hint = Every address this connector allows, on every port.
connector-ui-groups = Groups
connector-ui-no-groups = No groups yet. A group opens several devices at once.
connector-ui-devices = Devices
connector-ui-no-devices = No devices yet. Add those remotehub may reach one by one.
connector-ui-col-name = Name
connector-ui-col-address = Address
connector-ui-col-ports = Ports
connector-ui-col-groups = Groups
connector-ui-col-members = Devices
connector-ui-col-access = Access
connector-ui-row-closed = closed
connector-ui-row-open-until = open until { $until }
connector-ui-row-open = open without end
connector-ui-row-open-via = open through { $group }
connector-ui-hours = { $hours ->
    [one] 1 hour
   *[other] { $hours } hours
}
connector-ui-without-end = without end
connector-ui-open = Open
connector-ui-close-row = Close
connector-ui-remove = Remove
connector-ui-add-device = Add device
connector-ui-add-group = Add group
connector-ui-address-hint = An address, a range such as 10.0.0.0/24, or a host name.
connector-ui-ports-hint = Ports such as 22, 3389, 8000-8100.
connector-ui-error-name = A name has 1 to 64 characters.
connector-ui-error-address = { $value } is neither an address, a range nor a host name.
connector-ui-error-ports = { $value } is not a port or a range of ports.
connector-ui-error-no-ports = A device needs at least one port.
connector-ui-error-exists = { $name } exists already.
connector-ui-error-unknown = There is no { $name }.

connector-ui-requests = Requests from remotehub
connector-ui-requests-hint = Approving opens exactly the addresses and ports listed, for the time asked. Names and reasons are as remotehub states them.
connector-ui-request-asks = { $requester } asks for access for { $minutes ->
    [one] 1 minute
   *[other] { $minutes } minutes
}.
connector-ui-request-reason = Reason: { $reason }
connector-ui-col-target = Name in remotehub
connector-ui-col-port = Port
connector-ui-col-listed = Your list
connector-ui-listed-as = listed as { $name }
connector-ui-not-listed = not on your list
connector-ui-approve = Approve
connector-ui-refuse = Refuse
connector-ui-approved = Approved requests
connector-ui-approved-for = For { $requester }: { $reason }
connector-ui-error-request-gone = The request is no longer waiting: remotehub withdrew it, or someone answered it already.

connector-log-what-network = access
connector-log-what-device = the device { $name }
connector-log-what-group = the group { $name }
connector-log-what-request = the request { $id }
connector-log-opened-until = { $who } opened { $what } until { $until }.
connector-log-opened-until-cli = On the command line, opened { $what } until { $until }.
connector-log-opened-permanent = { $who } opened { $what } without end.
connector-log-opened-permanent-cli = On the command line, opened { $what } without end.
connector-log-closed = { $who } closed { $what }.
connector-log-closed-cli = On the command line, closed { $what }.
connector-log-expired = The time ran out: closed { $what }.
connector-log-command-line = The command line
connector-log-device-added = { $who } added the device { $name }: { $address }, ports { $ports }.
connector-log-device-removed = { $who } removed the device { $name }.
connector-log-group-added = { $who } added the group { $name }.
connector-log-group-removed = { $who } removed the group { $name }.
connector-log-request-received = remotehub asks, request { $id }: { $requester } wants { $targets } for { $minutes ->
    [one] 1 minute
   *[other] { $minutes } minutes
}. Reason: { $reason }
connector-log-request-approved = { $who } approved request { $id } of { $requester }: { $targets } until { $until }.
connector-log-request-refused = { $who } refused request { $id } of { $requester }: { $targets }.
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

connector-cli-token-prompt = Paste the connector's token from remotehub and press Enter:
connector-cli-installed = The Windows service remotehub-connector runs and reports to { $url }. Access is closed. Next, create a user for the web interface with: remotehub-connector user add NAME. Then open https://localhost:8480 on this server.
connector-cli-updated = The service runs release { $version } now.
connector-cli-uninstalled = The service and the program are removed.
connector-cli-data-kept = The data stays in { $path }; uninstall --purge removes it.

connector-cli-device-added = Device { $name } added. It is closed.
connector-cli-device-removed = Device { $name } removed.
connector-cli-group-added = Group { $name } added. It is closed.
connector-cli-group-removed = Group { $name } removed.
connector-cli-item-closed = { $name }: closed
connector-cli-item-open-until = { $name }: open until { $until }
connector-cli-item-open-permanent = { $name }: open without end
connector-cli-groups-label = Groups:
connector-cli-devices-label = Devices:
