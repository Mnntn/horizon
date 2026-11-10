//! Fault injection available only to scoped engine tests.
use super::DatapackRegistrySet;
impl DatapackRegistrySet {
    /// Exhaust the generation clock to exercise the engine publication failure path.
    #[doc(hidden)]
    pub fn exhaust_generation_for_testing(&mut self) {
        self.generation = u64::MAX;
    }
}
