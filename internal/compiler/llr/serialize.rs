// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

//! Serialization of a [`CompilationUnit`](super::CompilationUnit), for handing a compiled unit
//! to another process.
//!
//! The unit serializes on its own, without any state shared across fields.
//! The `Arc`s that the builtins provide are written as a reference to them,
//! and every other `Arc` is written by value, through serde's `rc` feature.
//! Both ends must be built from the same Slint version: the format isn't stable.

use crate::builtin_elements::BUILTIN_ELEMENTS;
use crate::langtype::{Enumeration, Struct, StructName};
use crate::native_class::NativeClass;
use crate::typeregister::{BUILTIN, builtin_structs};
use serde::de::Error as _;
use serde::ser::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::sync::Arc;

/// `#[serde(with = "native_class")]`: a class is one of [`BUILTIN_ELEMENTS`]' native classes,
/// written as its index there.
pub(crate) mod native_class {
    use super::*;

    pub fn serialize<S: Serializer>(
        class: &Arc<NativeClass>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let index = BUILTIN_ELEMENTS
            .native_classes
            .binary_search_by(|builtin| builtin.class_name.cmp(&class.class_name))
            .map_err(|_| S::Error::custom(format!("{} isn't builtin", class.class_name)))?;
        (index as u32).serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Arc<NativeClass>, D::Error> {
        let index = u32::deserialize(deserializer)?;
        BUILTIN_ELEMENTS
            .native_classes
            .get(index as usize)
            .cloned()
            .ok_or_else(|| D::Error::custom(format!("no native class {index}")))
    }
}

/// A value that the builtins provide, or any other one written by value.
#[derive(Serialize, Deserialize)]
enum Repr<Builtin, Other> {
    Builtin(Builtin),
    Other(Other),
}

/// `#[serde(with = "enumeration")]`: a builtin enum is written as its index in `BUILTIN.enums`,
/// so that a deserialized unit shares it with the compiler's.
pub(crate) mod enumeration {
    use super::*;

    pub fn serialize<S: Serializer>(
        enumeration: &Arc<Enumeration>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match BUILTIN.enums.all().position(|builtin| Arc::ptr_eq(builtin, enumeration)) {
            Some(index) => Repr::<u32, &Enumeration>::Builtin(index as u32),
            None => Repr::Other(&**enumeration),
        }
        .serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Arc<Enumeration>, D::Error> {
        match Repr::<u32, Enumeration>::deserialize(deserializer)? {
            Repr::Builtin(index) => BUILTIN
                .enums
                .all()
                .nth(index as usize)
                .cloned()
                .ok_or_else(|| D::Error::custom(format!("no builtin enum {index}"))),
            Repr::Other(enumeration) => Ok(Arc::new(enumeration)),
        }
    }
}

/// `#[serde(with = "structure")]`: a builtin struct is written as its name,
/// and read back from [`builtin_structs::get()`].
pub(crate) mod structure {
    use super::*;

    pub fn serialize<S: Serializer>(s: &Arc<Struct>, serializer: S) -> Result<S::Ok, S::Error> {
        match &s.name {
            StructName::Builtin(name) if Arc::ptr_eq(s, &builtin_structs::get(name)) => {
                Repr::<_, &Struct>::Builtin(name.clone())
            }
            _ => Repr::Other(&**s),
        }
        .serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Arc<Struct>, D::Error> {
        Ok(match Repr::<crate::langtype::BuiltinStruct, Struct>::deserialize(deserializer)? {
            Repr::Builtin(name) => builtin_structs::get(&name),
            Repr::Other(s) => Arc::new(s),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llr::CompilationUnit;

    fn compile(source: &str, debug_info: bool) -> CompilationUnit {
        let mut config =
            crate::CompilerConfiguration::new(crate::generator::OutputFormat::Interpreter);
        config.debug_info = debug_info;
        let mut diags = crate::diagnostics::BuildDiagnostics::default();
        let doc_node = crate::parser::parse(
            source.into(),
            Some(crate::source_path::SourcePath::new("t.slint")),
            &mut diags,
        );
        let (doc, diag, _) =
            spin_on::spin_on(crate::compile_syntax_node(doc_node, diags, config.clone()));
        assert!(!diag.has_errors(), "compile error: {:#?}", diag.to_string_vec());
        crate::llr::lower_to_item_tree::lower_to_item_tree(&doc, &config)
    }

    fn pretty(unit: &CompilationUnit) -> String {
        let mut out = String::new();
        crate::llr::pretty_print::pretty_print(unit, &mut out).unwrap();
        out
    }

    #[test]
    fn round_trip() {
        let source = r#"
            export enum Mode { a, b }
            export struct Point { x: int, y: int }
            export component Main inherits Window {
                in-out property <Mode> mode: Mode.b;
                in-out property <Point> point: { x: 1, y: 2 };
                in-out property <bool> is-b: mode == Mode.b;
                Rectangle { background: red; }
                Rectangle { background: point.x > 0 ? blue : green; }
                TouchArea { clicked => {
                    debug(mode);
                } }
            }
            // only-in-the-source
            "#;
        let unit = compile(source, true);

        let bytes = postcard::to_allocvec(&unit).unwrap();
        let restored: CompilationUnit = postcard::from_bytes(&bytes).unwrap();
        // Everything but the debug info came back
        assert_eq!(pretty(&restored), pretty(&compile(source, false)));
        assert!(!restored.has_debug_info);
        assert!(restored.sub_components.iter().all(|sc| sc.debug_info.is_none()));
        // The builtins came back as the builtins, or they'd now be written in full
        assert_eq!(postcard::to_allocvec(&restored).unwrap(), bytes);

        let root =
            &restored.sub_components[restored.public_components.first().unwrap().item_tree.root];
        let rectangles = root
            .items
            .iter()
            .filter(|item| item.ty.class_name == "Rectangle")
            .map(|item| &item.ty)
            .collect::<Vec<_>>();
        assert_eq!(rectangles.len(), 2);
        let compiled = unit.sub_components[unit.public_components.first().unwrap().item_tree.root]
            .items
            .iter()
            .find(|item| item.ty.class_name == "Rectangle")
            .unwrap();
        assert!(rectangles.iter().all(|class| Arc::ptr_eq(class, &compiled.ty)));

        let contains = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
        assert!(!contains(b"only-in-the-source"));
        assert!(!contains(b"t.slint"));
    }
}
