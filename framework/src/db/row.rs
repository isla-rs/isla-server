//! A serde `Deserializer` over one row: a map of column names to cells for
//! a struct, a sequence of cells for a tuple, the one cell for a value.

use std::fmt;

use serde::de::value::{SeqDeserializer, StrDeserializer};
use serde::de::{self, DeserializeOwned, DeserializeSeed, IntoDeserializer, MapAccess, SeqAccess, Unexpected, Visitor};

use super::{Error, Value};

/// `row` (with its `columns`) read into a `T`.
pub(super) fn read<T: DeserializeOwned>(columns: &[String], row: &[Value]) -> Result<T, Error> {
    T::deserialize(RowDe { columns, row }).map_err(|e| Error::Sql(format!("query_as: {}", e.0)))
}

#[derive(Debug)]
struct DeError(String);

impl fmt::Display for DeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DeError {}

impl de::Error for DeError {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        DeError(msg.to_string())
    }
}

struct RowDe<'a> {
    columns: &'a [String],
    row: &'a [Value],
}

impl<'a> RowDe<'a> {
    /// The row's one cell, for a row read as a single value.
    fn single(self) -> Result<CellDe<'a>, DeError> {
        match self.row {
            [value] => Ok(CellDe { value }),
            _ => Err(DeError(format!("a row of {} columns read as one value", self.row.len()))),
        }
    }
}

/// Row-level methods that read the row's single cell.
macro_rules! single {
    ($($method:ident),* $(,)?) => {
        $(fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
            self.single()?.$method(visitor)
        })*
    };
}

impl<'de> de::Deserializer<'de> for RowDe<'_> {
    type Error = DeError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        self.deserialize_map(visitor)
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        visitor.visit_map(Cells { columns: self.columns, row: self.row, next: 0 })
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, DeError> {
        self.deserialize_map(visitor)
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        visitor.visit_seq(Cells { columns: self.columns, row: self.row, next: 0 })
    }

    fn deserialize_tuple<V: Visitor<'de>>(self, _len: usize, visitor: V) -> Result<V::Value, DeError> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, DeError> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(self, name: &'static str, visitor: V) -> Result<V::Value, DeError> {
        self.single()?.deserialize_unit_struct(name, visitor)
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(self, name: &'static str, visitor: V) -> Result<V::Value, DeError> {
        self.single()?.deserialize_newtype_struct(name, visitor)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, DeError> {
        self.single()?.deserialize_enum(name, variants, visitor)
    }

    single!(
        deserialize_bool,
        deserialize_i8,
        deserialize_i16,
        deserialize_i32,
        deserialize_i64,
        deserialize_i128,
        deserialize_u8,
        deserialize_u16,
        deserialize_u32,
        deserialize_u64,
        deserialize_u128,
        deserialize_f32,
        deserialize_f64,
        deserialize_char,
        deserialize_str,
        deserialize_string,
        deserialize_bytes,
        deserialize_byte_buf,
        deserialize_option,
        deserialize_unit,
        deserialize_identifier,
        deserialize_ignored_any,
    );
}

/// The row's cells, as a map by column name or a sequence in order.
struct Cells<'a> {
    columns: &'a [String],
    row: &'a [Value],
    next: usize,
}

impl Cells<'_> {
    /// The next cell through `seed`, an error naming its column.
    fn cell<'de, S: DeserializeSeed<'de>>(&mut self, seed: S) -> Result<S::Value, DeError> {
        let i = self.next;
        self.next += 1;
        let column = self.columns.get(i).map_or("?", String::as_str);
        let value = self.row.get(i).ok_or_else(|| DeError(format!("column `{column}` has no cell")))?;
        seed.deserialize(CellDe { value }).map_err(|e| DeError(format!("column `{column}`: {e}")))
    }
}

impl<'de> MapAccess<'de> for Cells<'_> {
    type Error = DeError;

    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>, DeError> {
        let Some(column) = self.columns.get(self.next) else { return Ok(None) };
        let key: StrDeserializer<'_, DeError> = column.as_str().into_deserializer();
        seed.deserialize(key).map(Some)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, DeError> {
        self.cell(seed)
    }
}

impl<'de> SeqAccess<'de> for Cells<'_> {
    type Error = DeError;

    fn next_element_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<Option<T::Value>, DeError> {
        if self.next >= self.row.len() {
            return Ok(None);
        }
        self.cell(seed).map(Some)
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.row.len().saturating_sub(self.next))
    }
}

/// One cell.
struct CellDe<'a> {
    value: &'a Value,
}

impl CellDe<'_> {
    fn unexpected(&self) -> Unexpected<'_> {
        match self.value {
            Value::Null => Unexpected::Unit,
            Value::Int(i) => Unexpected::Signed(*i),
            Value::Real(f) => Unexpected::Float(*f),
            Value::Text(s) => Unexpected::Str(s),
            Value::Blob(b) => Unexpected::Bytes(b),
        }
    }
}

impl<'de> de::Deserializer<'de> for CellDe<'_> {
    type Error = DeError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match self.value {
            Value::Null => visitor.visit_unit(),
            Value::Int(i) => visitor.visit_i64(*i),
            Value::Real(f) => visitor.visit_f64(*f),
            Value::Text(s) => visitor.visit_str(s),
            Value::Blob(b) => visitor.visit_bytes(b),
        }
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match self.value {
            Value::Null => visitor.visit_none(),
            _ => visitor.visit_some(self),
        }
    }

    /// SQLite has no booleans: `0` and `1`.
    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match self.value {
            Value::Int(0) => visitor.visit_bool(false),
            Value::Int(1) => visitor.visit_bool(true),
            _ => Err(de::Error::invalid_value(self.unexpected(), &"0 or 1")),
        }
    }

    /// A blob into `Vec<u8>` (which asks for a sequence).
    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match self.value {
            Value::Blob(b) => visitor.visit_seq(SeqDeserializer::<_, DeError>::new(b.iter().copied())),
            _ => self.deserialize_any(visitor),
        }
    }

    fn deserialize_bytes<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match self.value {
            Value::Blob(b) => visitor.visit_bytes(b),
            Value::Text(s) => visitor.visit_bytes(s.as_bytes()),
            _ => self.deserialize_any(visitor),
        }
    }

    fn deserialize_byte_buf<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        self.deserialize_bytes(visitor)
    }

    /// A unit variant by its name, from text.
    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, DeError> {
        match self.value {
            Value::Text(s) => {
                let variant: StrDeserializer<'_, DeError> = s.as_str().into_deserializer();
                visitor.visit_enum(variant)
            }
            _ => Err(de::Error::invalid_type(self.unexpected(), &"text naming a variant")),
        }
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(self, _name: &'static str, visitor: V) -> Result<V::Value, DeError> {
        visitor.visit_newtype_struct(self)
    }

    serde::forward_to_deserialize_any! {
        i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        unit unit_struct tuple tuple_struct map struct identifier ignored_any
    }
}
