//! Text that remotehub renders on the server (ADR 0002): the admin CLI,
//! mails, and the site connector's web interface and command line.
//!
//! Every such text is a [`Message`]: a variant per message with typed
//! arguments, never a `String`. The Fluent catalogs `locales/{locale}/*.ftl`
//! are embedded in the binary; [`render`] picks the locale's catalog and
//! falls back to English. Tests hold every catalog to the messages declared
//! here: the same IDs, the same variables, and each renders without errors.

use std::sync::LazyLock;

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource, FluentValue};
use unic_langid::{LanguageIdentifier, langid};

/// A language remotehub has catalogs for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locale {
    En,
    De,
}

impl Locale {
    pub const ALL: [Locale; 2] = [Locale::En, Locale::De];

    fn id(self) -> LanguageIdentifier {
        match self {
            Locale::En => langid!("en"),
            Locale::De => langid!("de"),
        }
    }

    /// The locale of a language tag such as `de`, `de-CH` or `de_DE.UTF-8`.
    pub fn of_tag(tag: &str) -> Option<Locale> {
        let language = tag.split(['-', '_', '.', '@']).next()?.trim();
        match language.to_ascii_lowercase().as_str() {
            "en" => Some(Locale::En),
            "de" => Some(Locale::De),
            _ => None,
        }
    }

    /// The best locale for an HTTP `Accept-Language` header; English if none
    /// fits.
    pub fn from_accept_language(header: &str) -> Locale {
        let mut ranges: Vec<(f32, &str)> = header
            .split(',')
            .filter_map(|range| {
                let mut parts = range.split(';');
                let tag = parts.next()?.trim();
                let quality = parts
                    .find_map(|p| p.trim().strip_prefix("q="))
                    .map_or(Some(1.0), |q| q.trim().parse::<f32>().ok())?;
                (quality > 0.0).then_some((quality, tag))
            })
            .collect();
        // Stable: equal qualities keep the order the browser sent.
        ranges.sort_by(|a, b| b.0.total_cmp(&a.0));
        ranges
            .into_iter()
            .find_map(|(_, tag)| Locale::of_tag(tag))
            .unwrap_or(Locale::En)
    }

    /// The locale for a command line, from the POSIX variables `LC_ALL`,
    /// `LC_MESSAGES` and `LANG` as `lookup` returns them; English if none
    /// is set or remotehub has no catalog for it.
    pub fn from_posix(lookup: impl Fn(&str) -> Option<String>) -> Locale {
        ["LC_ALL", "LC_MESSAGES", "LANG"]
            .into_iter()
            .find_map(|name| lookup(name).filter(|value| !value.is_empty()))
            .and_then(|value| Locale::of_tag(&value))
            .unwrap_or(Locale::En)
    }
}

/// Declares the messages: the enum with typed arguments, and for the tests
/// each message's ID and variables and an instance to render.
macro_rules! messages {
    ($(
        $(#[$doc:meta])*
        $variant:ident = $id:literal { $($field:ident : $ty:ty),* $(,)? }
    )+) => {
        /// A text to render in a locale.
        #[derive(Debug, Clone, PartialEq)]
        pub enum Message {
            $($(#[$doc])* $variant { $($field: $ty),* },)+
        }

        impl Message {
            /// The message's ID in the catalogs.
            pub fn id(&self) -> &'static str {
                match self { $(Message::$variant { .. } => $id,)+ }
            }

            fn args(&self) -> FluentArgs<'_> {
                let mut args = FluentArgs::new();
                match self {
                    $(Message::$variant { $($field),* } => {
                        $(args.set(stringify!($field), FluentValue::from($field.clone()));)*
                    })+
                }
                args
            }

            /// Every message's ID with its variables.
            #[cfg(test)]
            const DECLARED: &[(&str, &[&str])] = &[$(($id, &[$(stringify!($field)),*]),)+];

            /// One instance of every message, with default arguments.
            #[cfg(test)]
            fn samples() -> Vec<Message> {
                vec![$(Message::$variant { $($field: <$ty>::default()),* },)+]
            }
        }
    };
}

messages! {
    /// `remotehub break-glass create` or `reset` succeeded. The password and
    /// the TOTP secret follow on their own lines, after the labels below,
    /// so they never pass through a message.
    BreakGlassReady = "break-glass-ready" { username: String }
    BreakGlassPasswordLabel = "break-glass-password-label" {}
    BreakGlassTotpSecretLabel = "break-glass-totp-secret-label" {}
    BreakGlassTotpUriLabel = "break-glass-totp-uri-label" {}
    BreakGlassSignIn = "break-glass-sign-in" { url: String }
    BreakGlassDeleted = "break-glass-deleted" { username: String }
    /// `remotehub account invite` succeeded. The one-time code follows on
    /// its own line, after the label, so it never passes through a message.
    AccountInvited = "account-invited" { email: String, expires: String }
    AccountCodeLabel = "account-code-label" {}
    AccountLinkLabel = "account-link-label" {}
    /// `remotehub setup-code` made a code. The link with the code follows
    /// on its own line, so it never passes through a message.
    SetupReady = "setup-ready" {}
    /// `remotehub setup-code` on an installation that is set up already.
    SetupComplete = "setup-complete" {}
    /// Mails (#145); the body is plain text, the HTML part is made from it.
    MailTestSubject = "mail-test-subject" {}
    MailTestBody = "mail-test-body" {}
    MailRecoverySubject = "mail-recovery-subject" {}
    MailRecoveryBody = "mail-recovery-body" { code: String }
    MailInvitationSubject = "mail-invitation-subject" {}
    MailInvitationBody = "mail-invitation-body" { name: String, link: String, code: String, expires: String }
    MailSignInCodeSubject = "mail-sign-in-code-subject" {}
    MailSignInCodeBody = "mail-sign-in-code-body" { name: String, link: String, code: String, expires: String }
    /// `remotehub verify-audit`: the hash chain holds.
    AuditIntact = "audit-intact" { entries: i64 }
    /// `remotehub verify-audit`: the chain breaks at entry `first`.
    AuditBroken = "audit-broken" { first: i64, entries: i64 }

    /// The site connector's web interface (#165). Arguments named `until`,
    /// `at`, `sent` and `received` get a marker that the interface replaces
    /// with an element the browser formats in its locale.
    ConnectorUiTitle = "connector-ui-title" {}
    ConnectorUiSignIn = "connector-ui-sign-in" {}
    ConnectorUiName = "connector-ui-name" {}
    ConnectorUiPassword = "connector-ui-password" {}
    ConnectorUiCode = "connector-ui-code" {}
    ConnectorUiCodeHint = "connector-ui-code-hint" {}
    ConnectorUiSignInFailed = "connector-ui-sign-in-failed" {}
    ConnectorUiLocked = "connector-ui-locked" { minutes: i64 }
    ConnectorUiSignOut = "connector-ui-sign-out" {}
    ConnectorUiSignedInAs = "connector-ui-signed-in-as" { name: String }
    ConnectorUiAccessFor = "connector-ui-access-for" { remotehub: String }
    ConnectorUiClosed = "connector-ui-closed" {}
    ConnectorUiOpenUntil = "connector-ui-open-until" { until: String }
    ConnectorUiOpenPermanent = "connector-ui-open-permanent" {}
    ConnectorUiChangedBy = "connector-ui-changed-by" { who: String, at: String }
    ConnectorUiChangedCli = "connector-ui-changed-cli" { at: String }
    ConnectorUiChangedExpiry = "connector-ui-changed-expiry" { at: String }
    ConnectorUiOpenFor = "connector-ui-open-for" { hours: i64 }
    ConnectorUiUntilField = "connector-ui-until-field" {}
    ConnectorUiOpenUntilButton = "connector-ui-open-until-button" {}
    ConnectorUiOpenPermanentButton = "connector-ui-open-permanent-button" {}
    ConnectorUiOpenPermanentHint = "connector-ui-open-permanent-hint" {}
    ConnectorUiClose = "connector-ui-close" {}
    ConnectorUiCloseHint = "connector-ui-close-hint" {}
    ConnectorUiInvalidUntil = "connector-ui-invalid-until" {}
    ConnectorUiConnections = "connector-ui-connections" {}
    ConnectorUiNoConnections = "connector-ui-no-connections" {}
    ConnectorUiColDevice = "connector-ui-col-device" {}
    ConnectorUiColUser = "connector-ui-col-user" {}
    ConnectorUiColSince = "connector-ui-col-since" {}
    ConnectorUiColTraffic = "connector-ui-col-traffic" {}
    ConnectorUiTraffic = "connector-ui-traffic" { sent: String, received: String }
    ConnectorUiUserHint = "connector-ui-user-hint" {}
    ConnectorUiLog = "connector-ui-log" {}
    ConnectorUiNoLog = "connector-ui-no-log" {}
    ConnectorUiColTime = "connector-ui-col-time" {}
    ConnectorUiColEvent = "connector-ui-col-event" {}
    /// Access per device and group (#180): the list and each row's state.
    ConnectorUiPartly = "connector-ui-partly" {}
    ConnectorUiNetwork = "connector-ui-network" {}
    ConnectorUiNetworkHint = "connector-ui-network-hint" {}
    ConnectorUiGroups = "connector-ui-groups" {}
    ConnectorUiNoGroups = "connector-ui-no-groups" {}
    ConnectorUiDevices = "connector-ui-devices" {}
    ConnectorUiNoDevices = "connector-ui-no-devices" {}
    ConnectorUiColName = "connector-ui-col-name" {}
    ConnectorUiColAddress = "connector-ui-col-address" {}
    ConnectorUiColPorts = "connector-ui-col-ports" {}
    ConnectorUiColGroups = "connector-ui-col-groups" {}
    ConnectorUiColMembers = "connector-ui-col-members" {}
    ConnectorUiColAccess = "connector-ui-col-access" {}
    ConnectorUiRowClosed = "connector-ui-row-closed" {}
    ConnectorUiRowOpenUntil = "connector-ui-row-open-until" { until: String }
    ConnectorUiRowOpen = "connector-ui-row-open" {}
    ConnectorUiRowOpenVia = "connector-ui-row-open-via" { group: String }
    ConnectorUiHours = "connector-ui-hours" { hours: i64 }
    ConnectorUiWithoutEnd = "connector-ui-without-end" {}
    ConnectorUiOpen = "connector-ui-open" {}
    ConnectorUiCloseRow = "connector-ui-close-row" {}
    ConnectorUiRemove = "connector-ui-remove" {}
    ConnectorUiAddDevice = "connector-ui-add-device" {}
    ConnectorUiAddGroup = "connector-ui-add-group" {}
    ConnectorUiAddressHint = "connector-ui-address-hint" {}
    ConnectorUiPortsHint = "connector-ui-ports-hint" {}
    ConnectorUiErrorName = "connector-ui-error-name" {}
    ConnectorUiErrorAddress = "connector-ui-error-address" { value: String }
    ConnectorUiErrorPorts = "connector-ui-error-ports" { value: String }
    ConnectorUiErrorNoPorts = "connector-ui-error-no-ports" {}
    ConnectorUiErrorExists = "connector-ui-error-exists" { name: String }
    ConnectorUiErrorUnknown = "connector-ui-error-unknown" { name: String }
    /// Requests for access from remotehub (#181). What remotehub states is
    /// shown as stated, next to the address an approval opens.
    ConnectorUiRequests = "connector-ui-requests" {}
    ConnectorUiRequestsHint = "connector-ui-requests-hint" {}
    ConnectorUiRequestAsks = "connector-ui-request-asks" { requester: String, minutes: i64 }
    ConnectorUiRequestReason = "connector-ui-request-reason" { reason: String }
    ConnectorUiColTarget = "connector-ui-col-target" {}
    ConnectorUiColPort = "connector-ui-col-port" {}
    ConnectorUiColListed = "connector-ui-col-listed" {}
    ConnectorUiListedAs = "connector-ui-listed-as" { name: String }
    ConnectorUiNotListed = "connector-ui-not-listed" {}
    ConnectorUiApprove = "connector-ui-approve" {}
    ConnectorUiRefuse = "connector-ui-refuse" {}
    ConnectorUiApproved = "connector-ui-approved" {}
    ConnectorUiApprovedFor = "connector-ui-approved-for" { requester: String, reason: String }
    ConnectorUiErrorRequestGone = "connector-ui-error-request-gone" {}
    /// The connector's journal as the web interface shows it. `what` is one
    /// of the `connector-log-what-*` messages.
    ConnectorLogWhatNetwork = "connector-log-what-network" {}
    ConnectorLogWhatDevice = "connector-log-what-device" { name: String }
    ConnectorLogWhatGroup = "connector-log-what-group" { name: String }
    ConnectorLogWhatRequest = "connector-log-what-request" { id: String }
    ConnectorLogOpenedUntil = "connector-log-opened-until" { who: String, what: String, until: String }
    ConnectorLogOpenedUntilCli = "connector-log-opened-until-cli" { what: String, until: String }
    ConnectorLogOpenedPermanent = "connector-log-opened-permanent" { who: String, what: String }
    ConnectorLogOpenedPermanentCli = "connector-log-opened-permanent-cli" { what: String }
    ConnectorLogClosed = "connector-log-closed" { who: String, what: String }
    ConnectorLogClosedCli = "connector-log-closed-cli" { what: String }
    ConnectorLogExpired = "connector-log-expired" { what: String }
    ConnectorLogCommandLine = "connector-log-command-line" {}
    ConnectorLogDeviceAdded = "connector-log-device-added" { who: String, name: String, address: String, ports: String }
    ConnectorLogDeviceRemoved = "connector-log-device-removed" { who: String, name: String }
    ConnectorLogGroupAdded = "connector-log-group-added" { who: String, name: String }
    ConnectorLogGroupRemoved = "connector-log-group-removed" { who: String, name: String }
    ConnectorLogRequestReceived = "connector-log-request-received" { id: String, requester: String, targets: String, minutes: i64, reason: String }
    ConnectorLogRequestApproved = "connector-log-request-approved" { who: String, id: String, requester: String, targets: String, until: String }
    ConnectorLogRequestRefused = "connector-log-request-refused" { who: String, id: String, requester: String, targets: String }
    ConnectorLogConnectionStarted = "connector-log-connection-started" { user: String, target: String }
    ConnectorLogConnectionEnded = "connector-log-connection-ended" { user: String, target: String, minutes: i64, sent: String, received: String }
    ConnectorLogConnectionRefused = "connector-log-connection-refused" { user: String, target: String, reason: String }
    ConnectorLogSignedIn = "connector-log-signed-in" { user: String, address: String }
    ConnectorLogSignInFailed = "connector-log-sign-in-failed" { user: String, address: String }
    ConnectorLogLocked = "connector-log-locked" { user: String, minutes: i64 }
    /// The site connector's command line (#165). Password and TOTP secret
    /// follow on their own lines, after the labels, so they never pass
    /// through a message.
    ConnectorCliClosed = "connector-cli-closed" {}
    ConnectorCliOpenUntil = "connector-cli-open-until" { until: String }
    ConnectorCliOpenPermanent = "connector-cli-open-permanent" {}
    ConnectorCliUserReady = "connector-cli-user-ready" { name: String }
    ConnectorCliPasswordLabel = "connector-cli-password-label" {}
    ConnectorCliTotpSecretLabel = "connector-cli-totp-secret-label" {}
    ConnectorCliTotpUriLabel = "connector-cli-totp-uri-label" {}
    ConnectorCliUserDeleted = "connector-cli-user-deleted" { name: String }
    ConnectorCliNoUsers = "connector-cli-no-users" {}
    ConnectorCliWithTotp = "connector-cli-with-totp" {}
    /// Installing the connector as a Windows service (#166).
    ConnectorCliTokenPrompt = "connector-cli-token-prompt" {}
    ConnectorCliInstalled = "connector-cli-installed" { url: String }
    ConnectorCliUpdated = "connector-cli-updated" { version: String }
    ConnectorCliUninstalled = "connector-cli-uninstalled" {}
    ConnectorCliDataKept = "connector-cli-data-kept" { path: String }
    /// The customer's list on the command line (#180).
    ConnectorCliDeviceAdded = "connector-cli-device-added" { name: String }
    ConnectorCliDeviceRemoved = "connector-cli-device-removed" { name: String }
    ConnectorCliGroupAdded = "connector-cli-group-added" { name: String }
    ConnectorCliGroupRemoved = "connector-cli-group-removed" { name: String }
    ConnectorCliItemClosed = "connector-cli-item-closed" { name: String }
    ConnectorCliItemOpenUntil = "connector-cli-item-open-until" { name: String, until: String }
    ConnectorCliItemOpenPermanent = "connector-cli-item-open-permanent" { name: String }
    ConnectorCliGroupsLabel = "connector-cli-groups-label" {}
    ConnectorCliDevicesLabel = "connector-cli-devices-label" {}
}

/// The catalog files of a locale, embedded in the binary.
fn sources(locale: Locale) -> &'static [&'static str] {
    match locale {
        Locale::En => &[
            include_str!("../locales/en/cli.ftl"),
            include_str!("../locales/en/mail.ftl"),
            include_str!("../locales/en/connector.ftl"),
        ],
        Locale::De => &[
            include_str!("../locales/de/cli.ftl"),
            include_str!("../locales/de/mail.ftl"),
            include_str!("../locales/de/connector.ftl"),
        ],
    }
}

fn bundle(locale: Locale) -> FluentBundle<FluentResource> {
    let mut bundle = FluentBundle::new_concurrent(vec![locale.id()]);
    // Unicode isolation marks around arguments would end up in terminals
    // and files, where nothing renders them.
    bundle.set_use_isolating(false);
    for source in sources(locale) {
        // The tests parse every catalog; a broken one never ships.
        let resource =
            FluentResource::try_new((*source).to_owned()).unwrap_or_else(|(resource, _)| resource);
        let _ = bundle.add_resource(resource);
    }
    bundle
}

static BUNDLES: LazyLock<[FluentBundle<FluentResource>; 2]> =
    LazyLock::new(|| [bundle(Locale::En), bundle(Locale::De)]);

fn bundle_of(locale: Locale) -> &'static FluentBundle<FluentResource> {
    &BUNDLES[match locale {
        Locale::En => 0,
        Locale::De => 1,
    }]
}

/// The message in `locale`, or the reason it cannot be rendered there.
fn try_render(locale: Locale, message: &Message) -> Result<String, String> {
    let bundle = bundle_of(locale);
    let pattern = bundle
        .get_message(message.id())
        .and_then(|m| m.value())
        .ok_or_else(|| format!("{} is missing in {:?}", message.id(), locale))?;
    let args = message.args();
    let mut errors = Vec::new();
    let text = bundle.format_pattern(pattern, Some(&args), &mut errors);
    if errors.is_empty() {
        Ok(text.into_owned())
    } else {
        Err(format!("{} in {:?}: {errors:?}", message.id(), locale))
    }
}

/// The message in `locale`; in English where the locale's catalog fails,
/// and as its ID if even that fails.
pub fn render(locale: Locale, message: &Message) -> String {
    try_render(locale, message)
        .or_else(|_| try_render(Locale::En, message))
        .unwrap_or_else(|_| message.id().to_owned())
}

#[cfg(test)]
mod tests;
