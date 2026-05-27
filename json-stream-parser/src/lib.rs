// This code is based on the json-stream-rust library (https://github.com/json-stream/json-stream-rust)
// Original code is MIT licensed
// Modified to fix escape character handling in strings
use std::mem;
use serde_json::{json, Value};

#[derive(Clone, Debug)]
enum ObjectStatus {
    Ready,
    StringQuoteOpen(bool),
    StringQuoteClose,
    Scalar {
        value_so_far: String,
    },
    ScalarNumber {
        value_so_far: String,
    },
    StartProperty,
    KeyQuoteOpen {
        key_so_far: String,
        escaped: bool,
    },
    KeyQuoteClose {
        key: String,
    },
    Colon {
        key: String,
    },
    ValueQuoteOpen {
        key: String,
        escaped: bool,
    },
    ValueQuoteClose,
    ValueScalar {
        key: String,
        value_so_far: String,
    },
    Closed,
}

/// Errors that can occur during streaming JSON parsing.
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    /// A key that was expected to have a string value has a different type.
    #[error("invalid value type for key `{0}`")]
    InvalidValueType(String),
    /// A numeric literal could not be parsed at the top level.
    #[error("invalid number: {0}")]
    InvalidNumber(String),
    /// A scalar value inside an object could not be parsed as JSON.
    #[error("invalid value for key `{0}`: {1}")]
    InvalidObjectValue(String, String),
    /// A character was encountered that does not fit the expected state.
    #[error("unexpected character `{char}`")]
    UnexpectedCharacter {
        char: char,
    },
}

fn add_char_into_object(
    object: &mut Value,
    current_status: &mut ObjectStatus,
    current_char: char,
) -> Result<(), ParseError> {
    match (&*object, &*current_status, current_char) {
        // --- Top-level string value ---
        (&Value::String(_), &ObjectStatus::StringQuoteOpen(true), '"') => {
            if let Value::String(s) = object {
                s.push('"');
            }
            if let ObjectStatus::StringQuoteOpen(ref mut escaped) = current_status {
                *escaped = false;
            }
        }
        (&Value::String(_), &ObjectStatus::StringQuoteOpen(false), '"') => {
            *current_status = ObjectStatus::StringQuoteClose;
        }
        (&Value::String(_), &ObjectStatus::StringQuoteOpen(true), c) => {
            if let Value::String(s) = object {
                s.push('\\');
                s.push(c);
            }
            if let ObjectStatus::StringQuoteOpen(ref mut escaped) = current_status {
                *escaped = false;
            }
        }
        (&Value::String(_), &ObjectStatus::StringQuoteOpen(false), '\\') => {
            if let ObjectStatus::StringQuoteOpen(ref mut escaped) = current_status {
                *escaped = true;
            }
        }
        (&Value::String(_), &ObjectStatus::StringQuoteOpen(false), c) => {
            if let Value::String(s) = object {
                s.push(c);
            }
        }

        // --- Object: key with escaped quote ---
        (&Value::Object(_), &ObjectStatus::KeyQuoteOpen { escaped: true, .. }, '"') => {
            if let ObjectStatus::KeyQuoteOpen { ref mut key_so_far, ref mut escaped } =
                current_status
            {
                key_so_far.push('"');
                *escaped = false;
            }
        }
        (&Value::Object(_), &ObjectStatus::KeyQuoteOpen { escaped: false, .. }, '"') => {
            if let ObjectStatus::KeyQuoteOpen { ref mut key_so_far, .. } = current_status {
                let key = mem::take(key_so_far);
                if let Value::Object(obj) = object {
                    obj.insert(key.clone(), Value::Null);
                }
                *current_status = ObjectStatus::KeyQuoteClose { key };
            }
        }
        // --- Object: key with escaped other char ---
        (&Value::Object(_), &ObjectStatus::KeyQuoteOpen { escaped: true, .. }, c) => {
            if let ObjectStatus::KeyQuoteOpen { ref mut key_so_far, ref mut escaped } =
                current_status
            {
                key_so_far.push('\\');
                key_so_far.push(c);
                *escaped = false;
            }
        }
        (&Value::Object(_), &ObjectStatus::KeyQuoteOpen { escaped: false, .. }, '\\') => {
            if let ObjectStatus::KeyQuoteOpen { ref mut escaped, .. } = current_status {
                *escaped = true;
            }
        }
        (&Value::Object(_), &ObjectStatus::KeyQuoteOpen { escaped: false, .. }, c) => {
            if let ObjectStatus::KeyQuoteOpen {
                ref mut key_so_far, ..
            } = current_status
            {
                key_so_far.push(c);
            }
        }

        // --- Object: value with escaped quote ---
        (&Value::Object(_), &ObjectStatus::ValueQuoteOpen { escaped: true, .. }, '"') => {
            if let ObjectStatus::ValueQuoteOpen { ref key, ref mut escaped } = current_status {
                if let Value::Object(obj) = object {
                    if let Some(Value::String(value)) = obj.get_mut(key) {
                        value.push('"');
                    }
                }
                *escaped = false;
            }
        }
        (&Value::Object(_), &ObjectStatus::ValueQuoteOpen { escaped: false, .. }, '"') => {
            *current_status = ObjectStatus::ValueQuoteClose;
        }
        (&Value::Object(_), &ObjectStatus::ValueQuoteOpen { escaped: true, .. }, c) => {
            if let ObjectStatus::ValueQuoteOpen { ref key, ref mut escaped } = current_status {
                if let Value::Object(obj) = object {
                    if let Some(Value::String(value)) = obj.get_mut(key) {
                        value.push('\\');
                        value.push(c);
                    }
                }
                *escaped = false;
            }
        }
        (&Value::Object(_), &ObjectStatus::ValueQuoteOpen { escaped: false, .. }, '\\') => {
            if let ObjectStatus::ValueQuoteOpen { ref mut escaped, .. } = current_status {
                *escaped = true;
            }
        }
        (&Value::Object(_), &ObjectStatus::ValueQuoteOpen { escaped: false, .. }, c) => {
            if let ObjectStatus::ValueQuoteOpen { ref key, .. } = current_status {
                if let Value::Object(obj) = object {
                    if let Some(Value::String(value)) = obj.get_mut(key) {
                        value.push(c);
                    } else {
                        return Err(ParseError::InvalidValueType(key.clone()));
                    }
                }
            }
        }

        // --- Top-level init ---
        (&Value::Null, &ObjectStatus::Ready, '"') => {
            *object = json!("");
            *current_status = ObjectStatus::StringQuoteOpen(false);
        }
        (&Value::Null, &ObjectStatus::Ready, '{') => {
            *object = json!({});
            *current_status = ObjectStatus::StartProperty;
        }

        (&Value::Null, &ObjectStatus::Ready, 't') => {
            *object = json!(true);
            *current_status = ObjectStatus::Scalar {
                value_so_far: "t".into(),
            };
        }
        (&Value::Bool(true), &ObjectStatus::Scalar { .. }, 'r') => {
            if let ObjectStatus::Scalar {
                ref mut value_so_far,
            } = current_status
            {
                if *value_so_far == "t" {
                    value_so_far.push('r');
                }
            }
        }
        (&Value::Bool(true), &ObjectStatus::Scalar { .. }, 'u') => {
            if let ObjectStatus::Scalar {
                ref mut value_so_far,
            } = current_status
            {
                if *value_so_far == "tr" {
                    value_so_far.push('u');
                }
            }
        }
        (&Value::Bool(true), &ObjectStatus::Scalar { .. }, 'e')
        | (&Value::Bool(false), &ObjectStatus::Scalar { .. }, 'e')
        | (&Value::Object(_), &ObjectStatus::ValueQuoteClose, '}') => {
            *current_status = ObjectStatus::Closed;
        }

        (&Value::Null, &ObjectStatus::Ready, 'f') => {
            *object = json!(false);
            *current_status = ObjectStatus::Scalar {
                value_so_far: "f".into(),
            };
        }
        (&Value::Bool(false), &ObjectStatus::Scalar { .. }, 'a') => {
            if let ObjectStatus::Scalar {
                ref mut value_so_far,
            } = current_status
            {
                if *value_so_far == "f" {
                    value_so_far.push('a');
                }
            }
        }
        (&Value::Bool(false), &ObjectStatus::Scalar { .. }, 'l') => {
            if let ObjectStatus::Scalar {
                ref mut value_so_far,
            } = current_status
            {
                if *value_so_far == "fa" {
                    value_so_far.push('l');
                }
            }
        }
        (&Value::Bool(false), &ObjectStatus::Scalar { .. }, 's') => {
            if let ObjectStatus::Scalar {
                ref mut value_so_far,
            } = current_status
            {
                if *value_so_far == "fal" {
                    value_so_far.push('s');
                }
            }
        }

        (&Value::Null, &ObjectStatus::Ready, 'n') => {
            *object = json!(null);
            *current_status = ObjectStatus::Scalar {
                value_so_far: "n".into(),
            };
        }
        (&Value::Null, &ObjectStatus::Scalar { .. }, 'u') => {
            if let ObjectStatus::Scalar {
                ref mut value_so_far,
            } = current_status
            {
                if *value_so_far == "n" {
                    value_so_far.push('u');
                }
            }
        }
        (&Value::Null, &ObjectStatus::Scalar { .. }, 'l') => {
            if let ObjectStatus::Scalar {
                ref mut value_so_far,
            } = current_status
            {
                if *value_so_far == "nu" {
                    value_so_far.push('l');
                } else if *value_so_far == "nul" {
                    *current_status = ObjectStatus::Closed;
                }
            }
        }

        // --- Numbers ---
        (&Value::Null, &ObjectStatus::Ready, c @ '0'..='9') => {
            if let Some(digit) = c.to_digit(10) {
                *object = Value::Number(digit.into());
            }
            *current_status = ObjectStatus::ScalarNumber {
                value_so_far: c.to_string(),
            };
        }
        (&Value::Null, &ObjectStatus::Ready, '-') => {
            *object = Value::Number(0.into());
            *current_status = ObjectStatus::ScalarNumber {
                value_so_far: "-".into(),
            };
        }
        (&Value::Number(_), &ObjectStatus::ScalarNumber { .. }, c @ '0'..='9') => {
            if let ObjectStatus::ScalarNumber {
                ref mut value_so_far,
            } = current_status
            {
                value_so_far.push(c);
                if let Value::Number(ref mut num) = object {
                    if value_so_far.contains('.') {
                        let parsed: f64 = value_so_far.parse().map_err(|e| {
                            ParseError::InvalidNumber(format!(
                                "invalid float: `{value_so_far}` ({e})"
                            ))
                        })?;
                        if let Some(json_number) = serde_json::Number::from_f64(parsed) {
                            *num = json_number;
                        }
                    } else {
                        let parsed: i64 = value_so_far.parse().map_err(|e| {
                            ParseError::InvalidNumber(format!(
                                "invalid integer: `{value_so_far}` ({e})"
                            ))
                        })?;
                        *num = parsed.into();
                    }
                }
            }
        }
        (&Value::Number(_), &ObjectStatus::ScalarNumber { .. }, '.') => {
            if let ObjectStatus::ScalarNumber {
                ref mut value_so_far,
            } = current_status
            {
                value_so_far.push('.');
            }
        }

        // --- Object structure ---
        (&Value::Object(_), &ObjectStatus::StartProperty, '"') => {
            *current_status = ObjectStatus::KeyQuoteOpen {
                key_so_far: String::new(),
                escaped: false,
            };
        }
        (&Value::Object(_), &ObjectStatus::KeyQuoteClose { .. }, ':') => {
            if let ObjectStatus::KeyQuoteClose { ref mut key } = current_status {
                let key = mem::take(key);
                *current_status = ObjectStatus::Colon { key };
            }
        }
        (&Value::Object(_), &ObjectStatus::Colon { .. }, ' ' | '\n' | '\t' | '\r') => {}
        (&Value::Object(_), &ObjectStatus::Colon { .. }, '"') => {
            if let ObjectStatus::Colon { ref mut key } = current_status {
                let key_str = mem::take(key);
                if let Value::Object(obj) = object {
                    obj.insert(key_str.clone(), json!(""));
                }
                *current_status = ObjectStatus::ValueQuoteOpen {
                    key: key_str,
                    escaped: false,
                };
            }
        }

        (&Value::Object(_), &ObjectStatus::Colon { .. }, char) => {
            if let ObjectStatus::Colon { ref mut key } = current_status {
                let key = mem::take(key);
                *current_status = ObjectStatus::ValueScalar {
                    key,
                    value_so_far: char.to_string(),
                };
            }
        }
        (&Value::Object(_), &ObjectStatus::ValueScalar { .. }, ',') => {
            if let ObjectStatus::ValueScalar {
                ref mut key,
                ref mut value_so_far,
            } = current_status
            {
                let key = mem::take(key);
                let value_str = mem::take(value_so_far);
                if let Value::Object(obj) = object {
                    match value_str.parse::<Value>() {
                        Ok(value) => {
                            obj.insert(key, value);
                        }
                        Err(e) => {
                            return Err(ParseError::InvalidObjectValue(key, e.to_string()));
                        }
                    }
                }
                *current_status = ObjectStatus::StartProperty;
            }
        }
        (&Value::Object(_), &ObjectStatus::ValueScalar { .. }, '}') => {
            if let ObjectStatus::ValueScalar {
                ref mut key,
                ref mut value_so_far,
            } = current_status
            {
                let key = mem::take(key);
                let value_str = mem::take(value_so_far);
                if let Value::Object(obj) = object {
                    match value_str.parse::<Value>() {
                        Ok(value) => {
                            obj.insert(key, value);
                        }
                        Err(e) => {
                            return Err(ParseError::InvalidObjectValue(key, e.to_string()));
                        }
                    }
                }
                *current_status = ObjectStatus::Closed;
            }
        }
        (&Value::Object(_), &ObjectStatus::ValueScalar { .. }, char) => {
            if let ObjectStatus::ValueScalar {
                ref mut value_so_far,
                ..
            } = current_status
            {
                value_so_far.push(char);
            }
        }

        (&Value::Object(_), &ObjectStatus::ValueQuoteClose, ',') => {
            *current_status = ObjectStatus::StartProperty;
        }
        (_, _, ' ' | '\n' | '\t' | '\r') => {}

        (_, _, c) => {
            return Err(ParseError::UnexpectedCharacter { char: c });
        }
    }
    Ok(())
}

/// Parse a complete JSON string in one shot.
///
/// Processes the input character-by-character through the streaming parser state machine.
///
/// # Errors
///
/// Returns [`ParseError`] if the input contains invalid JSON or unexpected characters
/// for the current parser state.
#[must_use]
pub fn parse_stream(json_string: &str) -> Result<Value, ParseError> {
    let mut out: Value = Value::Null;
    let mut current_status = ObjectStatus::Ready;
    for current_char in json_string.chars() {
        add_char_into_object(&mut out, &mut current_status, current_char)?;
    }
    Ok(out)
}

/// An incremental streaming JSON parser that processes one character at a time.
///
/// Create via [`JsonStreamParser::new()`], feed characters with [`add_char`](Self::add_char),
/// and retrieve the result with [`result`](Self::result).
///
/// # Example
///
/// ```
/// # use json_stream_parser::JsonStreamParser;
/// let mut parser = JsonStreamParser::new();
/// for c in r#"{"key": "value"}"#.chars() {
///     parser.add_char(c).unwrap();
/// }
/// let result = parser.result();
/// ```
#[derive(Debug)]
pub struct JsonStreamParser {
    object: Value,
    current_status: ObjectStatus,
}

impl Default for JsonStreamParser {
    fn default() -> Self {
        Self::new()
    }
}

impl JsonStreamParser {
    /// Create a new streaming parser in the initial state.
    #[must_use]
    pub fn new() -> JsonStreamParser {
        JsonStreamParser {
            object: Value::Null,
            current_status: ObjectStatus::Ready,
        }
    }

    /// Feed one character into the parser.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError`] if the character is invalid for the current state.
    pub fn add_char(&mut self, current_char: char) -> Result<(), ParseError> {
        add_char_into_object(&mut self.object, &mut self.current_status, current_char)
    }

    /// Borrow the current parsed value.
    #[must_use]
    pub fn result(&self) -> &Value {
        &self.object
    }
}

macro_rules! param_test {
    ($($name:ident: $string:expr, $value:expr)*) => {
    $(
        mod $name {
            #![allow(clippy::unwrap_used, clippy::unreadable_literal)]
            #[allow(unused_imports)]
            use super::{parse_stream, JsonStreamParser};
            #[allow(unused_imports)]
            use serde_json::{Value, json};

            #[test]
            fn simple() {
                let string: &str = $string;
                let value: Value = $value;
                let result = parse_stream(string);
                assert_eq!(result.unwrap(), value);
                let mut parser = JsonStreamParser::new();
                for c in string.chars() {
                    parser.add_char(c).unwrap();
                }
                assert_eq!(parser.result(), &value);
            }

            #[test]
            fn object_single_key_value() {
                let string = $string;
                let value = $value;
                let raw_json = format!("{{\"key\": {}}}", string);
                let expected = json!({"key": value});
                let result = parse_stream(&raw_json);
                assert_eq!(result.unwrap(), expected);
                let mut parser = JsonStreamParser::new();
                for c in raw_json.chars() {
                    parser.add_char(c).unwrap();
                }
                assert_eq!(parser.result(), &expected);
            }

            #[test]
            fn object_multiple_key_value() {
                let string = $string;
                let value = $value;
                let raw_json = format!("{{\"key1\": {}, \"key2\": {}}}", string, string);
                let expected = json!({"key1": value, "key2": value});
                let result = parse_stream(&raw_json);
                assert_eq!(result.unwrap(), expected);
                let mut parser = JsonStreamParser::new();
                for c in raw_json.chars() {
                    parser.add_char(c).unwrap();
                }
                assert_eq!(parser.result(), &expected);
            }

            #[test]
            fn object_multiple_key_value_with_blank_1() {
                let string = $string;
                let value = $value;
                let raw_json = format!("{{ \"key1\": {}, \"key2\": {}}}", string, string);
                let expected = json!({"key1": value, "key2": value});
                let result = parse_stream(&raw_json);
                assert_eq!(result.unwrap(), expected);
                let mut parser = JsonStreamParser::new();
                for c in raw_json.chars() {
                    parser.add_char(c).unwrap();
                }
                assert_eq!(parser.result(), &expected);
            }

            #[test]
            fn object_multiple_key_value_with_blank_2() {
                let string = $string;
                let value = $value;
                let raw_json = format!("{{\"key1\": {}, \"key2\": {} }}", string, string);
                let expected = json!({"key1": value, "key2": value});
                let result = parse_stream(&raw_json);
                assert_eq!(result.unwrap(), expected);
                let mut parser = JsonStreamParser::new();
                for c in raw_json.chars() {
                    parser.add_char(c).unwrap();
                }
                assert_eq!(parser.result(), &expected);
            }

            #[test]
            fn object_multiple_key_value_with_blank_3() {
                let string = $string;
                let value = $value;
                let raw_json = format!("{{ 
                    \"key1\": {} , 
                     \"key2\": {} 
                }}", string, string);
                let expected = json!({"key1": value, "key2": value});
                let result = parse_stream(&raw_json);
                assert_eq!(result.unwrap(), expected);
                let mut parser = JsonStreamParser::new();
                for c in raw_json.chars() {
                    parser.add_char(c).unwrap();
                }
                assert_eq!(parser.result(), &expected);
            }

            #[test]
            fn test_escaped_quotes_in_objects() {
                let raw_json = r#"{"key with \"quotes\"": "value with \"quotes\""}"#;
                let expected = json!({"key with \"quotes\"": "value with \"quotes\""});

                let result = parse_stream(raw_json);
                assert!(result.is_ok(), "Parse error: {:?}", result.err());
                assert_eq!(result.unwrap(), expected);

                let mut parser = JsonStreamParser::new();
                for c in raw_json.chars() {
                    assert!(parser.add_char(c).is_ok(), "Add char error");
                }
                assert_eq!(parser.result(), &expected);
            }
        }
    )*
    }
}

param_test! {
    null: r#"null"#, Value::Null
    true_value: r#"true"#, Value::Bool(true)
    false_value: r#"false"#, Value::Bool(false)
    empty_string: r#""""#, Value::String(String::new())
    single_character_string: r#""a""#, Value::String("a".to_string())
    string_with_spaces: r#""a b c""#, Value::String("a b c".to_string())
    string_with_space_at_end: r#""a b c ""#, Value::String("a b c ".to_string())
    string_with_space_at_start: r#"" a b c""#, Value::String(" a b c".to_string())
    string_with_space_at_start_and_end: r#"" a b c ""#, Value::String(" a b c ".to_string())
    number: r#"1234567890"#, Value::Number(1_234_567_890.into())
    single_digit_number: r#"1"#, Value::Number(1.into())
    number_with_spaces_at_start: r#" 1234567890"#, Value::Number(1_234_567_890.into())
    number_with_spaces_at_end: r#"1234567890 "#, Value::Number(1_234_567_890.into())
    number_with_spaces_at_start_and_end: r#" 1234567890 "#, Value::Number(1_234_567_890.into())
    negative_number: r#"-1234567890"#, Value::Number((-1_234_567_890i64).into())
    negative_single_digit_number: r#"-1"#, Value::Number((-1).into())
    zero: r#"0"#, Value::Number(0.into())
    float: r#"123.456"#, Value::Number(serde_json::Number::from_f64(123.456).unwrap())
    negative_float: r#"-123.456"#, Value::Number(serde_json::Number::from_f64(-123.456).unwrap())
    escaped_quotes: r#""he said \"hello\"""#, Value::String(r#"he said "hello""#.to_string())
}
