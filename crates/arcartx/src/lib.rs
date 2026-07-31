//! ArcartX configuration compatibility for Mythicraft.
//!
//! This crate deliberately does not depend on `mythicraft-client-services`: that crate is
//! currently excluded from the root workspace. The `protocol` module contains wire-compatible
//! DTOs and explicit conversion methods so the dependency can be added later without changing
//! the ArcartX-facing model.

mod dto;
mod model;
mod parser;

pub use dto::{
    ActionEnvelopeContext, ConversionError, UiActionDto, UiActionInputDto, UiActionTypeDto,
    UiOpenDto, UiUpdateDto,
};
pub use model::{
    ActionDefinition, ActionType, ArcartxDocument, Control, DocumentKind, PageConfig, ResourceRef,
    UiSettings, UiTask,
};
pub use parser::{
    parse_auto, parse_json, parse_yaml, Diagnostic, DiagnosticSeverity, InputFormat, ParseError,
    ParseReport,
};
