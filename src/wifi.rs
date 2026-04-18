use embassy_executor::Spawner;
use embassy_net::{Config, DhcpConfig, Runner, Stack, StackResources};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Timer};
use esp_hal::peripherals;
use esp_radio::wifi::{
    self, ControllerConfig, Interface, WifiController, WifiError,
    sta::StationConfig,
    Config as WifiConfig,
};
use log::{error, info};
use static_cell::StaticCell;

/// Static cell for network stack resources
static STACK_RESOURCES: StaticCell<StackResources<3>> = StaticCell::new();

/// Signal to request to stop WiFi
pub static STOP_WIFI_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

pub async fn connect_to_wifi(
    wifi: peripherals::WIFI<'static>,
    seed: u64,
    spawner: Spawner,
) -> Result<Stack<'static>, WifiError> {
    let station_config = StationConfig::default()
        .with_ssid(env!("WIFI_SSID"))
        .with_password(env!("WIFI_PSK").into());

    let controller_config = ControllerConfig::default()
        .with_initial_config(WifiConfig::Station(station_config));

    let (controller, interfaces) = wifi::new(wifi, controller_config)?;

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

    loop {
        if controller.is_connected() {
            controller.wait_for_disconnect_async().await.ok();
            Timer::after(Duration::from_millis(5000)).await;
        }

        info!("About to connect to {}...", env!("WIFI_SSID"));
        match controller.connect_async().await {
            Ok(_) => {
                info!("Connected to WiFi network");
                info!("Wait for request to stop wifi");
                STOP_WIFI_SIGNAL.wait().await;
                info!("Received signal to stop wifi");
                controller.disconnect_async().await.ok();
                break;
            }
            Err(error) => {
                error!("Failed to connect to WiFi network: {:?}", error);
                Timer::after(Duration::from_millis(5000)).await;
            }
        }
    }
    info!("Leave connection task");
    Ok(())
}
