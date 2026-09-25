//! Custom fields of shared credentials (#98), as KeePass has them: plain
//! ones are stored with the credential and shown to whoever sees it,
//! protected ones are sealed in the vault with the credential's version and
//! shown only with `reveal` (`api/reveal.rs`).

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

use super::catalog::invalid;
use super::problem::Problem;

/// More fields than this are not a credential any more.
const MAX_FIELDS: usize = 50;
const MAX_NAME: usize = 100;
const MAX_VALUE: usize = 10_000;

/// A field as a client sends it. A protected field without a value keeps
/// the one it has.
#[derive(Deserialize)]
pub struct FieldInput {
    name: String,
    #[serde(default)]
    protected: bool,
    value: Option<SecretString>,
}

/// A field as stored in `credentials.fields` and shown in the tree: a
/// protected one without its value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    pub name: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub protected: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

/// The checked fields of a change.
pub struct Checked {
    /// What `credentials.fields` holds from now on.
    pub stored: Vec<Field>,
    /// Protected fields with a new value, to seal.
    pub sealed: Vec<(String, SecretString)>,
    /// Protected fields that keep their value.
    pub kept: Vec<String>,
}

/// Where a protected field's value is sealed.
pub fn secret_name(name: &str) -> String {
    format!("field:{name}")
}

fn text(value: &str, limit: usize) -> bool {
    value.chars().count() <= limit && !value.chars().any(|c| c.is_control() && c != '\n')
}

pub fn check(input: &[FieldInput]) -> Result<Checked, Problem> {
    if input.len() > MAX_FIELDS {
        return Err(invalid("fields"));
    }
    let mut checked = Checked {
        stored: Vec::new(),
        sealed: Vec::new(),
        kept: Vec::new(),
    };
    for field in input {
        let name = field.name.trim();
        let duplicate = checked.stored.iter().any(|f| f.name == name);
        if name.is_empty() || name.contains('\n') || !text(name, MAX_NAME) || duplicate {
            return Err(invalid("fields"));
        }
        let value = field.value.as_ref().map(ExposeSecret::expose_secret);
        if value.is_some_and(|v| !text(v, MAX_VALUE)) {
            return Err(invalid("fields"));
        }
        checked.stored.push(Field {
            name: name.to_owned(),
            protected: field.protected,
            value: (!field.protected).then(|| value.unwrap_or_default().to_owned()),
        });
        match (field.protected, &field.value) {
            (true, Some(value)) => checked.sealed.push((name.to_owned(), value.clone())),
            (true, None) => checked.kept.push(name.to_owned()),
            (false, _) => {}
        }
    }
    Ok(checked)
}

impl Checked {
    /// Whether the sealed fields differ from those of `before`: new values,
    /// or protected fields added or gone.
    pub fn changes_secrets(&self, before: &[Field]) -> bool {
        let protected = |fields: &[Field]| -> Vec<String> {
            let mut names: Vec<String> = fields
                .iter()
                .filter(|f| f.protected)
                .map(|f| f.name.clone())
                .collect();
            names.sort();
            names
        };
        !self.sealed.is_empty() || protected(&self.stored) != protected(before)
    }

    /// Kept fields must have been protected fields before.
    pub fn keeps_only_what_was(&self, before: &[Field]) -> bool {
        self.kept
            .iter()
            .all(|name| before.iter().any(|f| f.protected && &f.name == name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str, protected: bool, value: Option<&str>) -> FieldInput {
        FieldInput {
            name: name.to_owned(),
            protected,
            value: value.map(|v| SecretString::from(v.to_owned())),
        }
    }

    #[test]
    fn protected_values_are_never_stored_in_plain() {
        let checked = check(&[
            input(" PIN ", true, Some("1234")),
            input("Account", false, Some("4711")),
            input("API key", true, None),
        ])
        .unwrap();
        assert_eq!(
            serde_json::to_value(&checked.stored).unwrap(),
            serde_json::json!([
                { "name": "PIN", "protected": true },
                { "name": "Account", "value": "4711" },
                { "name": "API key", "protected": true },
            ])
        );
        assert_eq!(checked.sealed.len(), 1);
        assert_eq!(checked.sealed[0].1.expose_secret(), "1234");
        assert_eq!(checked.kept, ["API key"]);
    }

    #[test]
    fn names_are_unique_and_plain() {
        for fields in [
            vec![input("", false, None)],
            vec![input("a\nb", false, None)],
            vec![input("x", false, None), input(" x", true, None)],
            vec![input(&"n".repeat(101), false, None)],
            vec![input("x", false, Some(&"v".repeat(10_001)))],
        ] {
            assert!(check(&fields).is_err());
        }
        let many: Vec<FieldInput> = (0..51)
            .map(|i| input(&i.to_string(), false, None))
            .collect();
        assert!(check(&many).is_err());
    }

    #[test]
    fn a_new_version_follows_protected_changes_only() {
        let before = vec![
            Field {
                name: "PIN".into(),
                protected: true,
                value: None,
            },
            Field {
                name: "Note".into(),
                protected: false,
                value: Some("a".into()),
            },
        ];
        let same = check(&[input("PIN", true, None), input("Note", false, Some("b"))]).unwrap();
        assert!(!same.changes_secrets(&before));
        assert!(same.keeps_only_what_was(&before));
        let gone = check(&[input("Note", false, Some("a"))]).unwrap();
        assert!(gone.changes_secrets(&before));
        let new_value = check(&[input("PIN", true, Some("9"))]).unwrap();
        assert!(new_value.changes_secrets(&before));
        let invented = check(&[input("PIN", true, None), input("TAN", true, None)]).unwrap();
        assert!(!invented.keeps_only_what_was(&before));
    }
}
