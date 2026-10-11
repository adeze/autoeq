//! HTML+WASM report renderer for AutoEQ.
//!
//! Plotly replacement: reports are a small HTML shell embedding a versioned
//! JSON payload plus WASM modules that create the plots client-side.
//! The default 2D renderer draws on Canvas 2D with geometry from
//! gpui-toolkit's `d3rs` (scales, axis layout, sankey layout, color schemes)
//! and needs no WebGPU. When an adapter is available, the shell accelerates
//! projected surface triangles with WebGPU in the same plots. Layout, axes,
//! colors, and interaction remain shared with the Canvas fallback.

pub mod assemble;
pub mod capture_quality;
pub mod crossover_config;
pub mod draw;
pub mod grid;
pub mod schema;
pub mod time_of_flight;

pub use capture_quality::{
    CAPTURE_QUALITY_INPUT_VERSION, CaptureQualityBand, CaptureQualityFinding, CaptureQualityMetric,
    CaptureQualityReport, CaptureQualityReportError, CaptureQualitySource, CaptureQualityStatus,
    CaptureQualityThreshold, CaptureStimulusMetadata, CaptureThresholdComparison,
    append_capture_quality_section, append_capture_quality_section_to_value,
};
#[doc(inline)]
pub use crossover_config::crossover_config_html;
pub use schema::*;
#[doc(inline)]
pub use time_of_flight::{time_of_flight_html, time_of_flight_rows};
