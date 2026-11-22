#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

//! Core Intermediate Representation (`ResourcePackIR`), mapping registry,
//! diagnostic reporting, and error types for `mc-rp-converter`.

pub mod error;
pub mod ir;
pub mod jsonc;
pub mod mapping;
pub mod report;

pub use error::CoreError;
pub use ir::{
    AnimationFrame, AnimationSpec, AttachableAsset, BedrockGeoVersion, DisplayTransform, Edition,
    ElementFace, ElementRotation, FaceDirection, LocaleTable, ModelAsset, ModelCategory,
    ModelElement, PackMetadata, PbrSourceMaps, ResolutionTier, ResourcePackIR, RotationAxis,
    RotationSnapMode, SoundAsset, SoundEntry, SoundEventDef, SoundRegistry, TextureAsset,
    TextureCategory, TexturePayload,
};
pub use jsonc::{
    decode_minecraft_text, parse_minecraft_json, sanitize_java_resource_path, sanitize_jsonc_bytes,
    sanitize_minecraft_json, sanitize_minecraft_json_bytes,
};
pub use mapping::{
    BedrockTextureTarget, BedrockVanillaIndex, MappingTable, PackVersionEntry, TextureMappingEntry,
    normalize_bedrock_engine_triplet,
};
pub use report::{
    AuditStatus, BrokenReference, ConversionReport, ConversionWarning, ForbiddenEntry,
    LossyConversion, PipelineStage, SemanticAuditReport, SkippedAsset, WarningSeverity,
};
