//! The sample event: one JSON object standing in for the events a program
//! will see in production.
//!
//! It buys two things. The obvious one is that a program can actually be run.
//! The subtle one is types: with no sample, the compiler is told the event is
//! of unknown shape, so `parse_json(.message)` is fallible because `.message`
//! *might* not be a string, and `.count + 1` is an error because `.count`
//! might be anything. Hand it a sample and those become facts — the field is a
//! string, or it does not exist at all — which is the difference between
//! "this might fail" and "this field is misspelled".

use vrl::compiler::state::ExternalEnv;
use vrl::value::{kind::Collection, Kind, Value};

/// Reads a sample event.
///
/// # Errors
///
/// If the text is not JSON, or is JSON but not an object. An event is an
/// object: the compiler assumes `.` is one, and a program written against a
/// bare array or string would be nonsense rather than merely unusual.
pub fn event_from_json(json: &str) -> Result<Value, String> {
    let parsed: serde_json::Value =
        serde_json::from_str(json).map_err(|error| format!("the sample event is not JSON: {error}"))?;

    if !parsed.is_object() {
        return Err(format!(
            "the sample event must be a JSON object, this is {}",
            match parsed {
                serde_json::Value::Null => "null",
                serde_json::Value::Bool(_) => "a boolean",
                serde_json::Value::Number(_) => "a number",
                serde_json::Value::String(_) => "a string",
                serde_json::Value::Array(_) => "an array",
                serde_json::Value::Object(_) => unreachable!("checked above"),
            },
        ));
    }

    Ok(Value::from(parsed))
}

/// The type environment a program compiles in, given what `.` holds.
///
/// The fields the sample has get the types the sample gives them. The fields
/// it does not have stay unknown — see [`open`] — and metadata stays unknown
/// too, since a sample says nothing about what Vector attaches at runtime.
#[must_use]
pub fn external_env(event: &Value) -> ExternalEnv {
    ExternalEnv::new_with_kind(open(Kind::from(event)), Kind::object(Collection::any()))
}

/// Says "and possibly other fields" about every object in a type.
///
/// `Kind::from` a sample event describes a *closed* object: these fields, with
/// these types, and nothing else. That reading is too strong for one example
/// event, and it breaks ordinary programs — `.event.original = …` becomes an
/// error, because the sample has no `.event` and so `.event` is known not to
/// be an object. Mapping an event into a new shape is most of what VRL is for,
/// and the sample cannot be allowed to forbid it.
///
/// Opening every object keeps everything the sample really tells us — that
/// `.message` is a string, which is what makes `downcase(.message)`
/// infallible — and drops the part it cannot know: what else an event might
/// carry. A field the sample lacks is then `any`, exactly as it is with no
/// sample at all, so nothing gets worse.
fn open(kind: Kind) -> Kind {
    let mut kind = kind;

    if let Some(object) = kind.as_object_mut() {
        object.set_unknown(Kind::any());
        for value in object.known_mut().values_mut() {
            *value = open(value.clone());
        }
    }

    if let Some(array) = kind.as_array_mut() {
        array.set_unknown(Kind::any());
        for value in array.known_mut().values_mut() {
            *value = open(value.clone());
        }
    }

    kind
}

/// The environment to compile in when there is no sample: `.` could be
/// anything, which is the honest default and the pessimistic one.
#[must_use]
pub fn unknown_env() -> ExternalEnv {
    ExternalEnv::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_object_is_read() {
        let event = event_from_json(r#"{"message":"hello","status":200}"#).expect("an object");
        let object = event.as_object().expect("the event is an object");

        assert_eq!(object.get("message"), Some(&Value::from("hello")));
        assert_eq!(object.get("status"), Some(&Value::from(200)));
    }

    #[test]
    fn anything_but_an_object_is_refused_by_name() {
        assert_eq!(
            event_from_json("[1, 2]").unwrap_err(),
            "the sample event must be a JSON object, this is an array",
        );
        assert!(event_from_json("nonsense").unwrap_err().starts_with("the sample event is not JSON"));
    }

    /// The shape reaching the compiler is the thing that matters, and the
    /// proof of it is in `run.rs`: a program that only compiles when
    /// `.message` is known to be a string.
    #[test]
    fn the_environment_carries_the_samples_shape() {
        let event = event_from_json(r#"{"message":"hello"}"#).expect("an object");

        assert!(Kind::from(&event).is_object());
        let _ = external_env(&event);
    }
}
