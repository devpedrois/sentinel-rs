pub mod cpu;
pub mod disk;
pub mod memory;
pub mod network;

#[allow(async_fn_in_trait)]
pub trait MetricCollector {
    type Output;
    type Error;

    async fn collect(&mut self) -> Result<Self::Output, Self::Error>;
}
