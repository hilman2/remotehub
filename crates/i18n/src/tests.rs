use std::collections::{BTreeMap, BTreeSet};

use fluent_syntax::ast::{self, Expression, InlineExpression, PatternElement};

use super::*;

/// Each message of a locale's catalogs with the variables it uses.
fn catalog(locale: Locale) -> BTreeMap<String, BTreeSet<String>> {
    let mut messages = BTreeMap::new();
    for source in sources(locale) {
        let resource = fluent_syntax::parser::parse(*source)
            .unwrap_or_else(|(_, errors)| panic!("{locale:?}: {errors:?}"));
        for entry in resource.body {
            match entry {
                ast::Entry::Message(message) => {
                    let mut variables = BTreeSet::new();
                    if let Some(pattern) = &message.value {
                        pattern_variables(pattern, &mut variables);
                    }
                    let id = message.id.name.to_owned();
                    assert!(
                        messages.insert(id.clone(), variables).is_none(),
                        "{locale:?}: {id} twice"
                    );
                }
                ast::Entry::Comment(_)
                | ast::Entry::GroupComment(_)
                | ast::Entry::ResourceComment(_) => {}
                other => panic!("{locale:?}: unexpected entry {other:?}"),
            }
        }
    }
    messages
}

fn pattern_variables(pattern: &ast::Pattern<&str>, variables: &mut BTreeSet<String>) {
    for element in &pattern.elements {
        if let PatternElement::Placeable { expression } = element {
            expression_variables(expression, variables);
        }
    }
}

fn expression_variables(expression: &Expression<&str>, variables: &mut BTreeSet<String>) {
    match expression {
        Expression::Inline(inline) => inline_variables(inline, variables),
        Expression::Select { selector, variants } => {
            inline_variables(selector, variables);
            for variant in variants {
                pattern_variables(&variant.value, variables);
            }
        }
    }
}

fn inline_variables(inline: &InlineExpression<&str>, variables: &mut BTreeSet<String>) {
    match inline {
        InlineExpression::VariableReference { id } => {
            variables.insert(id.name.to_owned());
        }
        InlineExpression::FunctionReference { arguments, .. } => {
            for argument in &arguments.positional {
                inline_variables(argument, variables);
            }
            for argument in &arguments.named {
                inline_variables(&argument.value, variables);
            }
        }
        InlineExpression::Placeable { expression } => expression_variables(expression, variables),
        _ => {}
    }
}

#[test]
fn every_catalog_has_exactly_the_declared_messages_and_variables() {
    let declared: BTreeMap<String, BTreeSet<String>> = Message::DECLARED
        .iter()
        .map(|(id, fields)| {
            let fields = fields.iter().map(|f| (*f).to_owned()).collect();
            ((*id).to_owned(), fields)
        })
        .collect();
    for locale in Locale::ALL {
        assert_eq!(catalog(locale), declared, "{locale:?}");
    }
}

#[test]
fn every_message_renders_in_every_locale() {
    for locale in Locale::ALL {
        for message in Message::samples() {
            let text = try_render(locale, &message).unwrap();
            assert!(!text.trim().is_empty(), "{locale:?} {}", message.id());
        }
    }
}

#[test]
fn arguments_and_plurals_come_out_as_written() {
    let intact = |entries| Message::AuditIntact { entries };
    assert_eq!(render(Locale::En, &intact(1)), "audit log intact: 1 entry");
    assert_eq!(
        render(Locale::En, &intact(1200)),
        "audit log intact: 1200 entries"
    );
    assert_eq!(
        render(Locale::De, &intact(1)),
        "Audit-Log intakt: 1 Eintrag"
    );
    let deleted = Message::BreakGlassDeleted {
        username: "emergency".to_owned(),
    };
    // No Unicode isolation marks around the name.
    assert_eq!(
        render(Locale::De, &deleted),
        "Notfallkonto emergency gelöscht."
    );
}

#[test]
fn locales_come_from_the_browser_with_english_last() {
    assert_eq!(
        Locale::from_accept_language("de-DE,de;q=0.9,en;q=0.8"),
        Locale::De
    );
    assert_eq!(
        Locale::from_accept_language("fr-FR, en;q=0.5, de;q=0.7"),
        Locale::De
    );
    assert_eq!(Locale::from_accept_language("en-GB,de;q=0.9"), Locale::En);
    assert_eq!(Locale::from_accept_language("de;q=0, en;q=0.1"), Locale::En);
    assert_eq!(Locale::from_accept_language("fr, it"), Locale::En);
    assert_eq!(Locale::from_accept_language(""), Locale::En);
    assert_eq!(Locale::from_accept_language("de;q=x"), Locale::En);
}

#[test]
fn locales_come_from_the_posix_variables_in_their_order() {
    let env = |pairs: &'static [(&'static str, &'static str)]| {
        move |name: &str| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        }
    };
    assert_eq!(
        Locale::from_posix(env(&[("LANG", "de_DE.UTF-8")])),
        Locale::De
    );
    assert_eq!(
        Locale::from_posix(env(&[("LANG", "de_DE.UTF-8"), ("LC_ALL", "C")])),
        Locale::En
    );
    assert_eq!(
        Locale::from_posix(env(&[("LANG", "en_US.UTF-8"), ("LC_MESSAGES", "de_AT")])),
        Locale::De
    );
    assert_eq!(
        Locale::from_posix(env(&[("LC_ALL", ""), ("LANG", "de")])),
        Locale::De
    );
    assert_eq!(Locale::from_posix(env(&[])), Locale::En);
}
