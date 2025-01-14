use core::str::FromStr;
use defmt::{error, info};
use embassy_executor::Spawner;
use embassy_net::{Stack, StackResources};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Timer};
use esp_hal::{peripheral::Peripheral, peripherals, rng::Rng};
use esp_wifi::wifi::{
    ClientConfiguration, Configuration, WifiController, WifiDevice, WifiError, WifiEvent,
    WifiStaDevice, WifiState,
};
use heapless::String;
use rand_core::RngCore;
use static_cell::StaticCell;

use crate::config::DEVICE_ID;

/// Static cell for network stack resources
static STACK_RESOURCES: StaticCell<StackResources<3>> = StaticCell::new();

/// Signal to request to stop WiFi
pub static STOP_WIFI_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

pub async fn connect_to_wifi(
    wifi: peripherals::WIFI,
    timer: esp_hal::timer::timg::Timer<
        esp_hal::timer::timg::TimerX<<esp_hal::peripherals::TIMG1 as Peripheral>::P>,
        esp_hal::Blocking,
    >,
    radio_clocks: peripherals::RADIO_CLK,
    mut rng: Rng,
    spawner: Spawner,
) -> Result<Stack<'static>, WifiError> {
    static INIT: StaticCell<esp_wifi::EspWifiController<'static>> = StaticCell::new();
    let init = INIT.init(esp_wifi::init(timer, rng, radio_clocks).unwrap());

    let (wifi_interface, controller) =
        esp_wifi::wifi::new_with_mode(init, wifi, WifiStaDevice).unwrap();

    // initialize network stack
    let mut dhcp_config = embassy_net::DhcpConfig::default();
    dhcp_config.hostname = Some(String::<32>::from_str(DEVICE_ID).unwrap());

    let seed = rng.next_u64();
    let config = embassy_net::Config::dhcpv4(dhcp_config);

    info!("Initialize network stack");
    let stack_resources: &'static mut _ = STACK_RESOURCES.init(StackResources::new());
    let (stack, runner) = embassy_net::new(wifi_interface, config, stack_resources, seed);

    spawner.spawn(connection(controller)).ok();
    spawner.spawn(net_task(runner)).ok();

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
async fn net_task(mut runner: embassy_net::Runner<'static, WifiDevice<'static, WifiStaDevice>>) {
    runner.run().await
}

/// Task for WiFi connection
///
/// This will wrap [`connection_fallible()`] and trap any error.
#[embassy_executor::task]
async fn connection(controller: WifiController<'static>) {
    if let Err(error) = connection_fallible(controller).await {
        error!("Cannot connect to WiFi: {}", error);
    }
}

async fn connection_fallible(mut controller: WifiController<'static>) -> Result<(), WifiError> {
    info!("Start connection task, device capabilities:");
    let caps = controller.capabilities().unwrap();
    caps.iter().for_each(|o| {
        info!("{:?}", o);
    });

    loop {
        if esp_wifi::wifi::wifi_state() == WifiState::StaConnected {
            // wait until we're no longer connected
            controller.wait_for_event(WifiEvent::StaDisconnected).await;
            Timer::after(Duration::from_millis(5000)).await
        }

        if !matches!(controller.is_started(), Ok(true)) {
            let ssid = env!("WIFI_SSID").try_into().unwrap();
            let password = env!("WIFI_PSK").try_into().unwrap();
            info!("Connecting to wifi with SSID: {}", ssid);
            let client_config = Configuration::Client(ClientConfiguration {
                ssid,
                password,
                ..Default::default()
            });
            controller.set_configuration(&client_config)?;
            info!("Starting WiFi controller");
            controller.start_async().await?;
            info!("WiFi controller started");
        }

        info!("About to connect to {}...", env!("WIFI_SSID"));
        match controller.connect_async().await {
            Ok(()) => {
                info!("Connected to WiFi network");
                info!("Wait for request to stop wifi");
                STOP_WIFI_SIGNAL.wait().await;
                info!("Received signal to stop wifi");
                controller.stop_async().await?;
                break;
            }
            Err(error) => {
                error!("Failed to connect to WiFi network: {}", error);
                Timer::after(Duration::from_millis(5000)).await;
            }
        }
    }
    info!("Leave connection task");
    Ok(())
}
