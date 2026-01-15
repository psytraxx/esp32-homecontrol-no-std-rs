use embassy_executor::Spawner;
use embassy_net::{Config, DhcpConfig, Runner, Stack, StackResources};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Timer};
use esp_hal::peripherals;
use esp_radio::{
    wifi::{
        self, ClientConfig, ModeConfig, WifiController, WifiDevice, WifiError, WifiEvent,
        WifiStaState,
    },
    Controller,
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
    static INIT: StaticCell<Controller<'static>> = StaticCell::new();
    let init = INIT.init(esp_radio::init().unwrap());

    let (controller, interfaces) = wifi::new(init, wifi, Default::default()).unwrap();

    let wifi_interface = interfaces.sta;

    // initialize network stack
    let dhcp_config = DhcpConfig::default();

    let config = Config::dhcpv4(dhcp_config);

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
async fn net_task(mut runner: Runner<'static, WifiDevice<'static>>) {
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
    info!("Start connection task, device capabilities:");
    let caps = controller.capabilities().unwrap();
    caps.iter().for_each(|o| {
        info!("{:?}", o);
    });

    loop {
        if wifi::sta_state() == WifiStaState::Connected {
            // wait until we're no longer connected
            controller.wait_for_event(WifiEvent::StaDisconnected).await;
            Timer::after(Duration::from_millis(5000)).await
        }

        if !matches!(controller.is_started(), Ok(true)) {
            let ssid = env!("WIFI_SSID").try_into().unwrap();
            let password = env!("WIFI_PSK").try_into().unwrap();
            info!("Connecting to wifi with SSID: {}", ssid);
            let client_config = ModeConfig::Client(
                ClientConfig::default()
                    .with_ssid(ssid)
                    .with_password(password),
            );

            controller.set_config(&client_config)?;
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
                error!("Failed to connect to WiFi network: {:?}", error);
                Timer::after(Duration::from_millis(5000)).await;
            }
        }
    }
    info!("Leave connection task");
    Ok(())
}
