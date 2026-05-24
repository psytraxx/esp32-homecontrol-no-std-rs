mod adc;
mod builder;
mod hardware;

pub use hardware::SensorPeripherals;

use embassy_sync::{blocking_mutex::raw::NoopRawMutex, channel::Sender};
use embassy_time::{Duration, Timer};
use log::info;

use crate::{config::AWAKE_DURATION_SECONDS, domain::SensorData};

#[embassy_executor::task]
pub async fn sensor_task(
    sender: Sender<'static, NoopRawMutex, SensorData, 3>,
    p: SensorPeripherals,
) {
    info!("Initializing sensor task");
    let mut hw = hardware::initialize_hardware(p).await;
    loop {
        let sensor_data = builder::collect_all_sensor_data(&mut hw).await;
        sender.send(sensor_data).await;
        Timer::after(Duration::from_secs(AWAKE_DURATION_SECONDS)).await;
    }
}
