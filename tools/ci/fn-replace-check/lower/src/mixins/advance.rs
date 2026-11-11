fn advance() {
    static SEEN: std::sync::Once = std::sync::Once::new();
    SEEN.call_once(|| fnr_lower::note("HEAD FixedStep::advance (horizon_client_lib)"));
}
