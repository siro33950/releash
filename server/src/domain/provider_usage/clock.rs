pub trait ProviderUsageClock: Send + Sync {
    fn now(&self) -> std::time::SystemTime;
}
