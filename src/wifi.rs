use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_net::{Config, DhcpConfig, Runner, Stack, StackResources};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Timer};
use esp_hal::peripherals;
use esp_radio::wifi::Config as WifiConfig;
use esp_radio::wifi::{ControllerConfig, Interface, WifiController, WifiError, sta::StationConfig};
use log::{error, info};
use static_cell::StaticCell;

use crate::config::{WIFI_RECONNECT_BACKOFF_MAX_MS, WIFI_RECONNECT_BACKOFF_START_MS};

/// Static cell for network stack resources
static STACK_RESOURCES: StaticCell<StackResources<3>> = StaticCell::new();

/// Signal to request to stop WiFi
pub static WIFI_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

pub async fn connect_to_wifi(
    wifi: peripherals::WIFI<'static>,
    seed: u64,
    spawner: Spawner,
) -> Result<Stack<'static>, WifiError> {
    let station_config = StationConfig::default()
        .with_ssid(env!("WIFI_SSID"))
        .with_password(env!("WIFI_PSK").into());

    let controller_config =
        ControllerConfig::default().with_initial_config(WifiConfig::Station(station_config));

    let (controller, interfaces) = esp_radio::wifi::new(wifi, controller_config)?;

    {
        use embassy_net::driver::Driver;
        let caps = interfaces.station.capabilities();
        info!(
            "WiFi driver capabilities: MTU={}, max_burst={:?}",
            caps.max_transmission_unit, caps.max_burst_size
        );
    }

    let dhcp_config = DhcpConfig::default();
    let config = Config::dhcpv4(dhcp_config);

    info!("Initialize network stack");
    let stack_resources: &'static mut _ = STACK_RESOURCES.init(StackResources::new());
    let (stack, runner) = embassy_net::new(interfaces.station, config, stack_resources, seed);

    spawner.spawn(connection(controller).expect("Unable to start controller"));
    spawner.spawn(net_task(runner).expect("Unable to start net task"));

    info!("Wait for network link");
    loop {
        if stack.is_link_up() {
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    info!("Wait for IP address");
    loop {
        if let Some(config) = stack.config_v4() {
            info!("Connected to WiFi with IP address {}", config.address);
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    Ok(stack)
}

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, Interface<'static>>) {
    runner.run().await
}

/// Task for WiFi connection
///
/// This will wrap [`connection_fallible()`] and trap any error.
#[embassy_executor::task]
async fn connection(controller: WifiController<'static>) {
    if let Err(error) = connection_fallible(controller).await {
        error!("Cannot connect to WiFi: {:?}", error);
    }
}

async fn connection_fallible(mut controller: WifiController<'static>) -> Result<(), WifiError> {
    info!("Start connection task");

    // Exponential backoff so a flaky AP can't trigger a tight reconnect storm
    // that keeps the radio (and its TX current spikes) busy on battery power.
    // Reset to the start value after a successful association.
    let mut backoff_ms = WIFI_RECONNECT_BACKOFF_START_MS;

    loop {
        if controller.is_connected() {
            controller.wait_for_disconnect_async().await.ok();
        }

        info!("About to connect to {}...", env!("WIFI_SSID"));
        match controller.connect_async().await {
            Ok(_) => {
                info!("Connected to WiFi network");
                backoff_ms = WIFI_RECONNECT_BACKOFF_START_MS;
                // Race: stop signal vs link drop. Reconnect if the AP drops us
                // rather than staying stuck with link_up=false until timeout.
                match select(WIFI_SIGNAL.wait(), controller.wait_for_disconnect_async()).await {
                    Either::First(_) => {
                        info!("Received signal to stop wifi");
                        controller.disconnect_async().await.ok();
                        break;
                    }
                    Either::Second(_) => {
                        info!("WiFi link dropped, reconnecting in {}ms...", backoff_ms);
                        Timer::after(Duration::from_millis(backoff_ms)).await;
                        backoff_ms = (backoff_ms * 2).min(WIFI_RECONNECT_BACKOFF_MAX_MS);
                    }
                }
            }
            Err(error) => {
                error!(
                    "Failed to connect to WiFi network: {:?} (retry in {}ms)",
                    error, backoff_ms
                );
                Timer::after(Duration::from_millis(backoff_ms)).await;
                backoff_ms = (backoff_ms * 2).min(WIFI_RECONNECT_BACKOFF_MAX_MS);
            }
        }
    }
    info!("Leave connection task");
    Ok(())
}
