//! The misplaced-key refusal, applied wherever a `Workflow` is deserialized.
//!
//! `Workflow` keeps an unknown top-level key out of its parse rather than
//! refusing it, so a `monitors:` or `phases:` written at the top level would be
//! dropped and the workflow would start unsupervised. `Workflow::from_content`
//! refuses those keys on the value it parses from a file, but a workflow also
//! arrives as a JSON body (`/agent/start`, `/workflows/save`, `/workflows/to-yaml`),
//! inside a deeplink, and as YAML or JSON a model hands the workflow and
//! schedule tools. All of them deserialize `Workflow` directly.
//!
//! [`RefuseMisplacedKeys`] wraps the deserializer the derived parse reads and
//! looks at each top-level key name as it goes past. It changes nothing else:
//! values are read by the same deserializer as before, so a YAML tag, serde_yaml's
//! reading of a plain scalar as a string, error positions and error paths are
//! exactly what the derived parse gives without it. Buffering the document into
//! a value first would have changed all four.

use std::fmt;

use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};

use super::supervision;

/// The deserializer of one `Workflow`, refusing a top-level key that belongs
/// inside `decision_settings` or `orchestration_plan`.
pub(super) struct RefuseMisplacedKeys<D>(pub(super) D);

impl<'de, D: Deserializer<'de>> Deserializer<'de> for RefuseMisplacedKeys<D> {
    type Error = D::Error;

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, D::Error> {
        self.0.deserialize_struct(name, fields, TopLevel(visitor))
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, D::Error> {
        self.0.deserialize_map(TopLevel(visitor))
    }

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, D::Error> {
        self.0.deserialize_any(TopLevel(visitor))
    }

    fn is_human_readable(&self) -> bool {
        self.0.is_human_readable()
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct enum identifier ignored_any
    }
}

/// The workflow's own visitor, handed a map whose keys are checked.
struct TopLevel<V>(V);

impl<'de, V: Visitor<'de>> Visitor<'de> for TopLevel<V> {
    type Value = V::Value;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        self.0.expecting(formatter)
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<V::Value, A::Error> {
        self.0.visit_map(CheckedKeys(map))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<V::Value, A::Error> {
        self.0.visit_seq(seq)
    }
}

/// The top-level map. Values pass straight through; only keys are wrapped.
struct CheckedKeys<A>(A);

impl<'de, A: MapAccess<'de>> MapAccess<'de> for CheckedKeys<A> {
    type Error = A::Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, A::Error> {
        self.0.next_key_seed(KeySeed(seed))
    }

    fn next_value_seed<S: DeserializeSeed<'de>>(&mut self, seed: S) -> Result<S::Value, A::Error> {
        self.0.next_value_seed(seed)
    }

    fn size_hint(&self) -> Option<usize> {
        self.0.size_hint()
    }
}

/// The seed of one top-level key.
struct KeySeed<K>(K);

impl<'de, K: DeserializeSeed<'de>> DeserializeSeed<'de> for KeySeed<K> {
    type Value = K::Value;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<K::Value, D::Error> {
        self.0.deserialize(KeyDeserializer(deserializer))
    }
}

/// The deserializer of one top-level key.
struct KeyDeserializer<D>(D);

impl<'de, D: Deserializer<'de>> Deserializer<'de> for KeyDeserializer<D> {
    type Error = D::Error;

    fn deserialize_identifier<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, D::Error> {
        self.0.deserialize_identifier(KeyName(visitor))
    }

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, D::Error> {
        self.0.deserialize_any(KeyName(visitor))
    }

    fn is_human_readable(&self) -> bool {
        self.0.is_human_readable()
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map struct enum ignored_any
    }
}

/// The derived field visitor, refusing a misplaced key name before it is
/// identified (and, being unknown, ignored).
///
/// It forwards what a derived field visitor reads a key from: a string, bytes
/// or a field index. Any other kind of key falls to `Visitor`'s defaults, which
/// refuse it with the same `expecting` the field visitor would.
struct KeyName<V>(V);

fn refuse<E: de::Error>(key: &str) -> Result<(), E> {
    supervision::refuse_misplaced_key(key).map_err(E::custom)
}

fn refuse_bytes<E: de::Error>(key: &[u8]) -> Result<(), E> {
    match std::str::from_utf8(key) {
        Ok(key) => refuse(key),
        Err(_) => Ok(()),
    }
}

impl<'de, V: Visitor<'de>> Visitor<'de> for KeyName<V> {
    type Value = V::Value;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        self.0.expecting(formatter)
    }

    fn visit_u64<E: de::Error>(self, index: u64) -> Result<V::Value, E> {
        self.0.visit_u64(index)
    }

    fn visit_str<E: de::Error>(self, key: &str) -> Result<V::Value, E> {
        refuse(key)?;
        self.0.visit_str(key)
    }

    fn visit_borrowed_str<E: de::Error>(self, key: &'de str) -> Result<V::Value, E> {
        refuse(key)?;
        self.0.visit_borrowed_str(key)
    }

    fn visit_string<E: de::Error>(self, key: String) -> Result<V::Value, E> {
        refuse(&key)?;
        self.0.visit_string(key)
    }

    fn visit_bytes<E: de::Error>(self, key: &[u8]) -> Result<V::Value, E> {
        refuse_bytes(key)?;
        self.0.visit_bytes(key)
    }

    fn visit_borrowed_bytes<E: de::Error>(self, key: &'de [u8]) -> Result<V::Value, E> {
        refuse_bytes(key)?;
        self.0.visit_borrowed_bytes(key)
    }

    fn visit_byte_buf<E: de::Error>(self, key: Vec<u8>) -> Result<V::Value, E> {
        refuse_bytes(&key)?;
        self.0.visit_byte_buf(key)
    }
}
