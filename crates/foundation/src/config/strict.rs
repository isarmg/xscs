//! Serde's field definitions remain authoritative. Rejecting ignored values
//! makes unknown fields strict even when a product omits deny_unknown_fields.

use crate::config::{ConfigError, ConfigSource, MAX_CONFIG_BYTES, Reason, child_path};
use serde::{
    Deserialize, Deserializer,
    de::{
        self, DeserializeOwned, DeserializeSeed, IntoDeserializer, MapAccess, SeqAccess, Visitor,
    },
};
use serde_json::{Map, Number, Value};
use std::{cell::RefCell, fmt, rc::Rc};

pub(super) fn parse_json(bytes: &[u8], source: ConfigSource) -> Result<Value, ConfigError> {
    if bytes.len() > MAX_CONFIG_BYTES {
        return Err(ConfigError::new(Reason::LimitExceeded, "", source));
    }
    let duplicate = Rc::new(RefCell::new(None));
    let mut parser = serde_json::Deserializer::from_slice(bytes);
    let seed = UniqueSeed {
        path: String::new(),
        duplicate: duplicate.clone(),
    };
    let value = seed.deserialize(&mut parser).and_then(|value| {
        parser.end()?;
        Ok(value)
    });
    value.map_err(|error| {
        let mut result = if let Some(path) = duplicate.borrow_mut().take() {
            ConfigError::new(Reason::DuplicateField, path, source)
        } else {
            ConfigError::new(Reason::InvalidSyntax, "", source)
        };
        result.line = Some(error.line());
        result.column = Some(error.column());
        result
    })
}

struct UniqueSeed {
    path: String,
    duplicate: Rc<RefCell<Option<String>>>,
}

impl<'de> DeserializeSeed<'de> for UniqueSeed {
    type Value = Value;
    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for UniqueSeed {
    type Value = Value;
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("JSON value")
    }
    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_bool<E>(self, value: bool) -> Result<Value, E> {
        Ok(Value::Bool(value))
    }
    fn visit_i64<E>(self, value: i64) -> Result<Value, E> {
        Ok(Value::Number(value.into()))
    }
    fn visit_u64<E>(self, value: u64) -> Result<Value, E> {
        Ok(Value::Number(value.into()))
    }
    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Value, E> {
        Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("finite JSON number required"))
    }
    fn visit_str<E>(self, value: &str) -> Result<Value, E> {
        Ok(Value::String(value.into()))
    }
    fn visit_string<E>(self, value: String) -> Result<Value, E> {
        Ok(Value::String(value))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Value, A::Error> {
        let mut values = Vec::new();
        loop {
            let seed = UniqueSeed {
                path: child_path(&self.path, &values.len().to_string()),
                duplicate: self.duplicate.clone(),
            };
            let Some(value) = sequence.next_element_seed(seed)? else {
                break;
            };
            values.push(value);
        }
        Ok(Value::Array(values))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Value, A::Error> {
        let mut map = Map::new();
        while let Some(key) = access.next_key::<String>()? {
            let path = child_path(&self.path, &key);
            if map.contains_key(&key) {
                *self.duplicate.borrow_mut() = Some(path);
                return Err(de::Error::custom("duplicate JSON field"));
            }
            let value = access.next_value_seed(UniqueSeed {
                path,
                duplicate: self.duplicate.clone(),
            })?;
            map.insert(key, value);
        }
        Ok(Value::Object(map))
    }
}

#[derive(Debug)]
pub(super) struct DataError {
    reason: Reason,
    path: Option<String>,
    relative: bool,
}

impl DataError {
    fn reason(reason: Reason) -> Self {
        Self {
            reason,
            path: None,
            relative: false,
        }
    }
    fn field(reason: Reason, field: &str) -> Self {
        Self {
            reason,
            path: Some(child_path("", field)),
            relative: true,
        }
    }
    fn at(mut self, path: &str) -> Self {
        if self.relative {
            self.path = Some(format!("{path}{}", self.path.take().unwrap_or_default()));
            self.relative = false;
        } else if self.path.is_none() {
            self.path = Some(path.into());
        }
        self
    }
}
impl fmt::Display for DataError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}", self.reason)
    }
}
impl std::error::Error for DataError {}
impl de::Error for DataError {
    fn custom<T: fmt::Display>(_message: T) -> Self {
        Self::reason(Reason::InvalidValue)
    }
    fn invalid_type(_value: de::Unexpected<'_>, _expected: &dyn de::Expected) -> Self {
        Self::reason(Reason::TypeMismatch)
    }
    fn invalid_value(_value: de::Unexpected<'_>, _expected: &dyn de::Expected) -> Self {
        Self::reason(Reason::InvalidValue)
    }
    fn invalid_length(_length: usize, _expected: &dyn de::Expected) -> Self {
        Self::reason(Reason::InvalidValue)
    }
    fn unknown_variant(_variant: &str, _expected: &'static [&'static str]) -> Self {
        Self::reason(Reason::InvalidValue)
    }
    fn unknown_field(field: &str, _expected: &'static [&'static str]) -> Self {
        Self::field(Reason::UnknownField, field)
    }
    fn missing_field(field: &'static str) -> Self {
        Self::field(Reason::MissingField, field)
    }
    fn duplicate_field(field: &'static str) -> Self {
        Self::field(Reason::DuplicateField, field)
    }
}

pub(super) fn deserialize<T: DeserializeOwned>(
    value: Value,
    source: ConfigSource,
) -> Result<T, ConfigError> {
    T::deserialize(Node {
        value,
        path: String::new(),
    })
    .map_err(|error| ConfigError::new(error.reason, error.path.unwrap_or_default(), source))
}

struct Node {
    value: Value,
    path: String,
}

impl<'de> Deserializer<'de> for Node {
    type Error = DataError;
    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DataError> {
        let path = self.path;
        let result = match self.value {
            Value::Null => visitor.visit_unit(),
            Value::Bool(value) => visitor.visit_bool(value),
            Value::Number(number) => {
                if let Some(value) = number.as_u64() {
                    visitor.visit_u64(value)
                } else if let Some(value) = number.as_i64() {
                    visitor.visit_i64(value)
                } else {
                    visitor.visit_f64(number.as_f64().expect("finite JSON number"))
                }
            }
            Value::String(value) => visitor.visit_string(value),
            Value::Array(values) => visitor.visit_seq(Sequence {
                values: values.into_iter(),
                path: path.clone(),
                index: 0,
            }),
            Value::Object(values) => visitor.visit_map(Object {
                values: values.into_iter(),
                path: path.clone(),
                pending: None,
            }),
        };
        result.map_err(|error: DataError| error.at(&path))
    }
    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DataError> {
        let path = self.path.clone();
        (if self.value.is_null() {
            visitor.visit_none()
        } else {
            visitor.visit_some(self)
        })
        .map_err(|error: DataError| error.at(&path))
    }
    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, DataError> {
        let path = self.path.clone();
        visitor
            .visit_newtype_struct(self)
            .map_err(|error: DataError| error.at(&path))
    }
    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, DataError> {
        let path = self.path;
        let result = match self.value {
            Value::String(value) => {
                visitor.visit_enum(de::value::StringDeserializer::<DataError>::new(value))
            }
            Value::Object(values) if values.len() == 1 => {
                let (variant, value) = values.into_iter().next().expect("one entry");
                visitor.visit_enum(Enum {
                    variant,
                    value,
                    path: path.clone(),
                })
            }
            _ => return Err(DataError::reason(Reason::TypeMismatch).at(&path)),
        };
        result.map_err(|error: DataError| error.at(&path))
    }
    fn deserialize_ignored_any<V: Visitor<'de>>(self, _visitor: V) -> Result<V::Value, DataError> {
        Err(DataError::reason(Reason::UnknownField).at(&self.path))
    }
    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, DataError> {
        if !self.value.is_object() {
            return Err(DataError::reason(Reason::TypeMismatch).at(&self.path));
        }
        self.deserialize_any(visitor)
    }
    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 char str string bytes byte_buf
        unit unit_struct seq tuple tuple_struct map identifier
    }
}

struct Sequence {
    values: std::vec::IntoIter<Value>,
    path: String,
    index: usize,
}
impl<'de> SeqAccess<'de> for Sequence {
    type Error = DataError;
    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, DataError> {
        let Some(value) = self.values.next() else {
            return Ok(None);
        };
        let path = child_path(&self.path, &self.index.to_string());
        self.index += 1;
        seed.deserialize(Node {
            value,
            path: path.clone(),
        })
        .map(Some)
        .map_err(|error| error.at(&path))
    }
    fn size_hint(&self) -> Option<usize> {
        Some(self.values.len())
    }
}

struct Object {
    values: serde_json::map::IntoIter,
    path: String,
    pending: Option<(String, Value)>,
}
impl<'de> MapAccess<'de> for Object {
    type Error = DataError;
    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, DataError> {
        let Some((key, value)) = self.values.next() else {
            return Ok(None);
        };
        let path = child_path(&self.path, &key);
        self.pending = Some((path.clone(), value));
        seed.deserialize(key.into_deserializer())
            .map(Some)
            .map_err(|error: DataError| {
                // Serde's deny_unknown_fields error already includes this key.
                // Other key conversion failures still belong to the key itself.
                if error.relative {
                    error.at(&self.path)
                } else {
                    error.at(&path)
                }
            })
    }
    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, DataError> {
        let (path, value) = self
            .pending
            .take()
            .expect("Serde requests a map value after its key");
        seed.deserialize(Node {
            value,
            path: path.clone(),
        })
        .map_err(|error| error.at(&path))
    }
    fn size_hint(&self) -> Option<usize> {
        Some(self.values.len())
    }
}

struct Enum {
    variant: String,
    value: Value,
    path: String,
}
impl<'de> de::EnumAccess<'de> for Enum {
    type Error = DataError;
    type Variant = Node;
    fn variant_seed<V: DeserializeSeed<'de>>(self, seed: V) -> Result<(V::Value, Node), DataError> {
        let path = child_path(&self.path, &self.variant);
        let variant = seed.deserialize(de::value::StringDeserializer::<DataError>::new(
            self.variant,
        ))?;
        Ok((
            variant,
            Node {
                value: self.value,
                path,
            },
        ))
    }
}
impl<'de> de::VariantAccess<'de> for Node {
    type Error = DataError;
    fn unit_variant(self) -> Result<(), DataError> {
        <()>::deserialize(self)
    }
    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value, DataError> {
        let path = self.path.clone();
        seed.deserialize(self).map_err(|error| error.at(&path))
    }
    fn tuple_variant<V: Visitor<'de>>(
        self,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, DataError> {
        self.deserialize_any(visitor)
    }
    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, DataError> {
        self.deserialize_struct("", fields, visitor)
    }
}
