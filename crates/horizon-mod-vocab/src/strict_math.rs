//! Established StrictMath operations, pure on native and wasm, with identical bits.
//! Trigonometric sin/cos/tan, exp and pow are not provided.
pub use horizon_core::jmath::fdlibm::{cbrt, hypot, log, log10};
pub use horizon_core::jmath::{acos, asin, atan, atan2};
#[cfg(test)]
#[path = "strict_math_tests.rs"]
mod tests;
