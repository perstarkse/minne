// This code is based on the json-stream-rust library (https://github.com/json-stream/json-stream-rust)
// Original code is MIT licensed
// Modified to fix escape character handling in strings
use serde_json::{json, Value};

#[derive(Clone, Debug)]
enum ObjectStatus {
    Ready,
    StringQuoteOpen(bool),
    StringQuoteClose,
    Scalar {
        value_so_far: Vec<char>,
    },
    ScalarNumber {
        value_so_far: Vec<char>,
    },
    StartProperty,
    KeyQuoteOpen {
        key_so_far: Vec<char>,
        escaped: bool,
    },
    KeyQuoteClose {
        key: Vec<char>,
    },
    Colon {
        key: Vec<char>,
    },
    ValueQuoteOpen {
        key: Vec<char>,
        escaped: bool,
    },
    ValueQuoteClose,
    ValueScalar {
        key: Vec<char>,
        value_so_far: Vec<char>,
    },
    Closed,
}

fn add_char_into_object(
    object: &mut Value,
    current_status: &mut ObjectStatus,
    current_char: char,
) -> Result<(), String> {
    match (&*object, &*current_status, current_char) {
        (&Value::String(_), &ObjectStatus::StringQuoteOpen(true), '"') => {
            if let Value::String(str) = object {
                str.push('"');
            }
            *current_status = ObjectStatus::StringQuoteOpen(false);
        }
        (&Value::String(_), &ObjectStatus::StringQuoteOpen(false), '"') => {
            *current_status = ObjectStatus::StringQuoteClose;
        }
        (&Value::String(_), &ObjectStatus::StringQuoteOpen(true), c) => {
            if let Value::String(str) = object {
                str.push('\\');
                str.push(c);
            }
            *current_status = ObjectStatus::StringQuoteOpen(false);
        }
        (&Value::String(_), &ObjectStatus::StringQuoteOpen(false), '\\') => {
            *current_status = ObjectStatus::StringQuoteOpen(true);
        }
        (&Value::String(_), &ObjectStatus::StringQuoteOpen(false), c) => {
            if let Value::String(str) = object {
                str.push(c);
            }
        }

        (&Value::Object(_), &ObjectStatus::KeyQuoteOpen { escaped: true, .. }, '"') => {
            if let ObjectStatus::KeyQuoteOpen {
                ref mut key_so_far, ..
            } = current_status
            {
                key_so_far.push('"');
                *current_status = ObjectStatus::KeyQuoteOpen {
                    key_so_far: key_so_far.clone(),
                    escaped: false,
                };
            }
        }
        (&Value::Object(_), &ObjectStatus::KeyQuoteOpen { escaped: false, .. }, '"') => {
            if let ObjectStatus::KeyQuoteOpen { ref key_so_far, .. } = current_status {
                let key = key_so_far.iter().collect::<String>();
                if let Value::Object(obj) = object {
                    obj.insert(key, Value::Null);
                }
                *current_status = ObjectStatus::KeyQuoteClose {
                    key: key_so_far.clone(),
                };
            }
        }
        (&Value::Object(_), &ObjectStatus::KeyQuoteOpen { escaped: true, .. }, c) => {
            if let ObjectStatus::KeyQuoteOpen {
                ref mut key_so_far, ..
            } = current_status
            {
                key_so_far.push('\\');
                key_so_far.push(c);
                *current_status = ObjectStatus::KeyQuoteOpen {
                    key_so_far: key_so_far.clone(),
                    escaped: false,
                };
            }
        }
        (&Value::Object(_), &ObjectStatus::KeyQuoteOpen { escaped: false, .. }, '\\') => {
            if let ObjectStatus::KeyQuoteOpen { ref key_so_far, .. } = current_status {
                *current_status = ObjectStatus::KeyQuoteOpen {
                    key_so_far: key_so_far.clone(),
                    escaped: true,
                };
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

        (&Value::Object(_), &ObjectStatus::ValueQuoteOpen { escaped: true, .. }, '"') => {
            if let ObjectStatus::ValueQuoteOpen { ref key, .. } = current_status {
                let key_str = key.iter().collect::<String>();
                if let Value::Object(obj) = object {
                    if let Some(Value::String(value)) = obj.get_mut(&key_str) {
                        value.push('"');
                    }
                }
                *current_status = ObjectStatus::ValueQuoteOpen {
                    key: key.clone(),
                    escaped: false,
                };
            }
        }
        (&Value::Object(_), &ObjectStatus::ValueQuoteOpen { escaped: false, .. }, '"') => {
            *current_status = ObjectStatus::ValueQuoteClose;
        }
        (&Value::Object(_), &ObjectStatus::ValueQuoteOpen { escaped: true, .. }, c) => {
            if let ObjectStatus::ValueQuoteOpen { ref key, .. } = current_status {
                let key_str = key.iter().collect::<String>();
                if let Value::Object(obj) = object {
                    if let Some(Value::String(value)) = obj.get_mut(&key_str) {
                        value.push('\\');
                        value.push(c);
                    }
                }
                *current_status = ObjectStatus::ValueQuoteOpen {
                    key: key.clone(),
                    escaped: false,
                };
            }
        }
        (&Value::Object(_), &ObjectStatus::ValueQuoteOpen { escaped: false, .. }, '\\') => {
            if let ObjectStatus::ValueQuoteOpen { ref key, .. } = current_status {
                *current_status = ObjectStatus::ValueQuoteOpen {
                    key: key.clone(),
                    escaped: true,
                };
            }
        }
        (&Value::Object(_), &ObjectStatus::ValueQuoteOpen { escaped: false, .. }, c) => {
            if let ObjectStatus::ValueQuoteOpen { ref key, .. } = current_status {
                let key_str = key.iter().collect::<String>();
                if let Value::Object(obj) = object {
                    if let Some(Value::String(value)) = obj.get_mut(&key_str) {
                        value.push(c);
                    } else {
                        return Err(format!("Invalid value type for key {}", key_str));
                    }
                }
            }
        }

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
                value_so_far: vec!['t'],
            };
        }
        (&Value::Bool(true), &ObjectStatus::Scalar { .. }, 'r') => {
            if let ObjectStatus::Scalar {
                ref mut value_so_far,
            } = current_status
            {
                if *value_so_far == vec!['t'] {
                    value_so_far.push('r');
                }
            }
        }
        (&Value::Bool(true), &ObjectStatus::Scalar { .. }, 'u') => {
            if let ObjectStatus::Scalar {
                ref mut value_so_far,
            } = current_status
            {
                if *value_so_far == vec!['t', 'r'] {
                    value_so_far.push('u');
                }
            }
        }
        (&Value::Bool(true), &ObjectStatus::Scalar { .. }, 'e') => {
            *current_status = ObjectStatus::Closed;
        }

        (&Value::Null, &ObjectStatus::Ready, 'f') => {
            *object = json!(false);
            *current_status = ObjectStatus::Scalar {
                value_so_far: vec!['f'],
            };
        }
        (&Value::Bool(false), &ObjectStatus::Scalar { .. }, 'a') => {
            if let ObjectStatus::Scalar {
                ref mut value_so_far,
            } = current_status
            {
                if *value_so_far == vec!['f'] {
                    value_so_far.push('a');
                }
            }
        }
        (&Value::Bool(false), &ObjectStatus::Scalar { .. }, 'l') => {
            if let ObjectStatus::Scalar {
                ref mut value_so_far,
            } = current_status
            {
                if *value_so_far == vec!['f', 'a'] {
                    value_so_far.push('l');
                }
            }
        }
        (&Value::Bool(false), &ObjectStatus::Scalar { .. }, 's') => {
            if let ObjectStatus::Scalar {
                ref mut value_so_far,
            } = current_status
            {
                if *value_so_far == vec!['f', 'a', 'l'] {
                    value_so_far.push('s');
                }
            }
        }
        (&Value::Bool(false), &ObjectStatus::Scalar { .. }, 'e') => {
            *current_status = ObjectStatus::Closed;
        }

        (&Value::Null, &ObjectStatus::Ready, 'n') => {
            *object = json!(null);
            *current_status = ObjectStatus::Scalar {
                value_so_far: vec!['n'],
            };
        }
        (&Value::Null, &ObjectStatus::Scalar { .. }, 'u') => {
            if let ObjectStatus::Scalar {
                ref mut value_so_far,
            } = current_status
            {
                if *value_so_far == vec!['n'] {
                    value_so_far.push('u');
                }
            }
        }
        (&Value::Null, &ObjectStatus::Scalar { .. }, 'l') => {
            if let ObjectStatus::Scalar {
                ref mut value_so_far,
            } = current_status
            {
                if *value_so_far == vec!['n', 'u'] {
                    value_so_far.push('l');
                } else if *value_so_far == vec!['n', 'u', 'l'] {
                    *current_status = ObjectStatus::Closed;
                }
            }
        }

        (&Value::Null, &ObjectStatus::Ready, c @ '0'..='9') => {
            *object = Value::Number(c.to_digit(10).unwrap().into());
            *current_status = ObjectStatus::ScalarNumber {
                value_so_far: vec![c],
            };
        }
        (&Value::Null, &ObjectStatus::Ready, '-') => {
            *object = Value::Number(0.into());
            *current_status = ObjectStatus::ScalarNumber {
                value_so_far: vec!['-'],
            };
        }
        (&Value::Number(_), &ObjectStatus::ScalarNumber { .. }, c @ '0'..='9') => {
            if let ObjectStatus::ScalarNumber {
                ref mut value_so_far,
            } = current_status
            {
                value_so_far.push(c);
                if let Value::Number(ref mut num) = object {
                    if value_so_far.contains(&'.') {
                        let parsed_number = value_so_far
                            .iter()
                            .collect::<String>()
                            .parse::<f64>()
                            .unwrap();
                        if let Some(json_number) = serde_json::Number::from_f64(parsed_number) {
                            *num = json_number;
                        }
                    } else {
                        let parsed_number = value_so_far
                            .iter()
                            .collect::<String>()
                            .parse::<i64>()
                            .unwrap();
                        *num = parsed_number.into();
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

        (&Value::Object(_), &ObjectStatus::StartProperty, '"') => {
            *current_status = ObjectStatus::KeyQuoteOpen {
                key_so_far: vec![],
                escaped: false,
            };
        }
        (&Value::Object(_), &ObjectStatus::KeyQuoteClose { .. }, ':') => {
            if let ObjectStatus::KeyQuoteClose { ref key } = current_status {
                *current_status = ObjectStatus::Colon { key: key.clone() };
            }
        }
        (&Value::Object(_), &ObjectStatus::Colon { .. }, ' ' | '\n') => {}
        (&Value::Object(_), &ObjectStatus::Colon { .. }, '"') => {
            if let ObjectStatus::Colon { ref key } = current_status {
                let key_str = key.iter().collect::<String>();
                if let Value::Object(obj) = object {
                    obj.insert(key_str, json!(""));
                }
                *current_status = ObjectStatus::ValueQuoteOpen {
                    key: key.clone(),
                    escaped: false,
                };
            }
        }

        (&Value::Object(_), &ObjectStatus::Colon { .. }, char) => {
            if let ObjectStatus::Colon { ref key } = current_status {
                *current_status = ObjectStatus::ValueScalar {
                    key: key.clone(),
                    value_so_far: vec![char],
                };
            }
        }
        (&Value::Object(_), &ObjectStatus::ValueScalar { .. }, ',') => {
            if let ObjectStatus::ValueScalar {
                ref key,
                ref value_so_far,
            } = current_status
            {
                let key_str = key.iter().collect::<String>();
                let value_str = value_so_far.iter().collect::<String>();
                if let Value::Object(obj) = object {
                    match value_str.parse::<Value>() {
                        Ok(value) => {
                            obj.insert(key_str, value);
                        }
                        Err(e) => return Err(format!("Invalid value for key {}: {}", key_str, e)),
                    }
                }
                *current_status = ObjectStatus::StartProperty;
            }
        }
        (&Value::Object(_), &ObjectStatus::ValueScalar { .. }, '}') => {
            if let ObjectStatus::ValueScalar {
                ref key,
                ref value_so_far,
            } = current_status
            {
                let key_str = key.iter().collect::<String>();
                let value_str = value_so_far.iter().collect::<String>();
                if let Value::Object(obj) = object {
                    match value_str.parse::<Value>() {
                        Ok(value) => {
                            obj.insert(key_str, value);
                        }
                        Err(e) => return Err(format!("Invalid value for key {}: {}", key_str, e)),
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
        (&Value::Object(_), &ObjectStatus::ValueQuoteClose, '}') => {
            *current_status = ObjectStatus::Closed;
        }

        (_, _, ' ' | '\n') => {}

        (val, st, c) => {
            return Err(format!(
                "Invalid character {} status: {:?} value: {:?}",
                c, st, val
            ));
        }
    }
    Ok(())
}

#[cfg(debug_assertions)]
pub fn parse_stream(json_string: &str) -> Result<Value, String> {
    let mut out: Value = Value::Null;
    let mut current_status = ObjectStatus::Ready;
    for current_char in json_string.chars() {
        println!(
            "variables: {:?} {:?} {:?}",
            out,
            current_status.clone(),
            current_char.to_string()
        );
        add_char_into_object(&mut out, &mut current_status, current_char)?
    }
    Ok(out)
}

#[cfg(not(debug_assertions))]
pub fn parse_stream(json_string: &str) -> Result<Value, String> {
    let mut out: Value = Value::Null;
    let mut current_status = ObjectStatus::Ready;
    for current_char in json_string.chars() {
        if let Err(e) = add_char_into_object(&mut out, &mut current_status, current_char) {
            return Err(e);
        }
    }
    return Ok(out);
}

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
    pub fn new() -> JsonStreamParser {
        JsonStreamParser {
            object: Value::Null,
            current_status: ObjectStatus::Ready,
        }
    }
    pub fn add_char(&mut self, current_char: char) -> Result<(), String> {
        add_char_into_object(&mut self.object, &mut self.current_status, current_char)
    }
    pub fn get_result(&self) -> &Value {
        &self.object
    }
}

macro_rules! param_test {
    ($($name:ident: $string:expr, $value:expr)*) => {
    $(
        mod $name {
            #[allow(unused_imports)]
            use super::{parse_stream, JsonStreamParser};
            #[allow(unused_imports)]
            use serde_json::{Value, json};

            #[test]
            fn simple() {
                let string: &str = $string;
                let value: Value = $value;
                let result = parse_stream(&string);
                assert_eq!(result.unwrap(), value);
                let mut parser = JsonStreamParser::new();
                for c in string.chars() {
                    parser.add_char(c).unwrap();
                }
                assert_eq!(parser.get_result(), &value);
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
                assert_eq!(parser.get_result(), &expected);
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
                assert_eq!(parser.get_result(), &expected);
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
                assert_eq!(parser.get_result(), &expected);
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
                assert_eq!(parser.get_result(), &expected);
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
                assert_eq!(parser.get_result(), &expected);
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
                assert_eq!(parser.get_result(), &expected);
            }
        }
    )*
    }
}

param_test! {
    null: r#"null"#, Value::Null
    true_value: r#"true"#, Value::Bool(true)
    false_value: r#"false"#, Value::Bool(false)
    empty_string: r#""""#, Value::String("".to_string())
    single_character_string: r#""a""#, Value::String("a".to_string())
    string_with_spaces: r#""a b c""#, Value::String("a b c".to_string())
    string_with_space_at_end: r#""a b c ""#, Value::String("a b c ".to_string())
    string_with_space_at_start: r#"" a b c""#, Value::String(" a b c".to_string())
    string_with_space_at_start_and_end: r#"" a b c ""#, Value::String(" a b c ".to_string())
    number: r#"1234567890"#, Value::Number(1234567890.into())
    single_digit_number: r#"1"#, Value::Number(1.into())
    number_with_spaces_at_start: r#" 1234567890"#, Value::Number(1234567890.into())
    number_with_spaces_at_end: r#"1234567890 "#, Value::Number(1234567890.into())
    number_with_spaces_at_start_and_end: r#" 1234567890 "#, Value::Number(1234567890.into())
    negative_number: r#"-1234567890"#, Value::Number((-1234567890).into())
    negative_single_digit_number: r#"-1"#, Value::Number((-1).into())
    zero: r#"0"#, Value::Number(0.into())
    float: r#"123.456"#, Value::Number(serde_json::Number::from_f64(123.456).unwrap())
    negative_float: r#"-123.456"#, Value::Number(serde_json::Number::from_f64(-123.456).unwrap())
    escaped_quotes: r#""he said \"hello\"""#, Value::String(r#"he said "hello""#.to_string())
}
