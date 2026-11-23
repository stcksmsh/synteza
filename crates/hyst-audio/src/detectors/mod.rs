//! Layer 2 section detectors (build, break, drop), ported line for line from
//! `src/audio/worklet/brain/{build-detector,break-detector,drop-detector,novelty}.ts`.
//! Faithful ports first; tuning against real audio is a later, separate job.

pub mod break_detector;
pub mod build_detector;
pub mod drop_detector;
pub mod novelty;

pub use break_detector::{BreakDetector, BreakDetectorOutput, BreakEvent};
pub use build_detector::BuildDetector;
pub use drop_detector::{DropDetector, DropDetectorDebug, DropDetectorFeatures, DropEvent};
