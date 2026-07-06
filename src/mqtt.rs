use alloc::{
    format,
    string::{String, ToString},
};
use core::{num::NonZero, num::ParseIntError, str};
use embassy_net::{
    Stack,
    dns::{DnsQueryType, Error as DnsError},
    tcp::{ConnectError, TcpSocket},
};
use embassy_time::{Instant, with_deadline};
use log::{error, info, warn};
use rust_mqtt::{
    Bytes,
    buffer::AllocBuffer,
    client::{
        Client, MqttError,
        event::Event,
        options::{
            ConnectOptions, PublicationOptions, RetainHandling, SubscriptionOptions, TopicReference,
        },
    },
    config::{KeepAlive, SessionExpiryInterval},
    types::{MqttBinary, MqttString, QoS, ReasonCode, TopicName},
};
use serde_json::{Value, json};
use static_cell::StaticCell;
use strum::IntoEnumIterator;

use crate::{
    DISCOVERY_MESSAGES_SENT,
    config::{
        DEVICE_ID, HOMEASSISTANT_DISCOVERY_TOPIC_PREFIX, HOMEASSISTANT_SENSOR_TOPIC,
        HOMEASSISTANT_SWITCH_TOPIC, MQTT_PUBLISH_ENABLED,
    },
    domain::{Sensor, SensorData},
};

const BUFFER_SIZE: usize = 4096;

struct MqttResources {
    rx_buffer: [u8; BUFFER_SIZE],
    tx_buffer: [u8; BUFFER_SIZE],
    alloc_buffer: AllocBuffer,
}

static RESOURCES: StaticCell<MqttResources> = StaticCell::new();

type MqttClientImpl<'a> = Client<'a, TcpSocket<'a>, AllocBuffer, 1, 1, 1, 1>;

pub struct MqttSession<'a> {
    client: MqttClientImpl<'a>,
    pump_set_topic: String,
}

/// Resolve the broker, open the TCP socket and connect the MQTT session.
/// Called once per wake cycle — there is no reconnect loop; on failure the
/// device simply sleeps and retries on the next wake.
pub async fn connect(stack: Stack<'static>) -> Result<MqttSession<'static>, Error> {
    let resources = RESOURCES.init(MqttResources {
        rx_buffer: [0u8; BUFFER_SIZE],
        tx_buffer: [0u8; BUFFER_SIZE],
        alloc_buffer: AllocBuffer,
    });

    let mut socket = TcpSocket::new(stack, &mut resources.rx_buffer, &mut resources.tx_buffer);

    let host_addr = stack
        .dns_query(env!("MQTT_HOSTNAME"), DnsQueryType::A)
        .await?
        .first()
        .copied()
        .ok_or(Error::DnsNoRecords)?;

    let port = env!("MQTT_PORT").parse()?;
    let socket_addr = (host_addr, port);

    info!("Connecting to MQTT server...");
    socket.connect(socket_addr).await?;
    info!("Connected to MQTT server");

    let options = ConnectOptions {
        user_name: Some(MqttString::try_from(env!("MQTT_USERNAME")).unwrap()),
        password: Some(MqttBinary::try_from(env!("MQTT_PASSWORD")).unwrap()),
        clean_start: true,
        keep_alive: KeepAlive::Seconds(NonZero::new(60).unwrap()),
        session_expiry_interval: SessionExpiryInterval::Seconds(60),
        will: None,
        ..Default::default()
    };

    let mut client = Client::<'_, _, _, 1, 1, 1, 1>::new(&mut resources.alloc_buffer);

    if let Err(e) = client
        .connect(
            socket,
            &options,
            Some(MqttString::try_from(DEVICE_ID).unwrap()),
        )
        .await
    {
        error!("Failed to connect to server: {:?}", e);
        return Err(e.into());
    }

    info!("MQTT Broker connected");

    Ok(MqttSession {
        client,
        pump_set_topic: pump_set_topic(),
    })
}

impl MqttSession<'_> {
    /// Publish discovery messages (first boot only) and the sensor state topics,
    /// honoring the MQTT_PUBLISH_ENABLED development gate.
    pub async fn publish(&mut self, sensor_data: &SensorData) -> Result<(), Error> {
        if !MQTT_PUBLISH_ENABLED {
            info!("MQTT publishing disabled, skipping");
            return Ok(());
        }
        self.publish_discovery_topics().await?;
        self.publish_sensor_data(sensor_data).await
    }

    /// Subscribe to the pump command topic. The retained message is always
    /// delivered on subscribe, so an ON set while the device was asleep is never
    /// missed. Callers must establish the overflow state *before* subscribing.
    pub async fn subscribe_to_pump_commands(&mut self) -> Result<(), Error> {
        let sub_options = SubscriptionOptions {
            // Always deliver retained message on subscribe so a pending ON
            // set while the device was asleep is never missed.
            retain_handling: RetainHandling::AlwaysSend,
            retain_as_published: false,
            no_local: false,
            qos: QoS::AtMostOnce,
            ..Default::default()
        };

        let topic =
            TopicName::new_unchecked(MqttString::try_from(self.pump_set_topic.as_str()).unwrap());
        self.client.subscribe(topic.into(), sub_options).await?;

        info!("Subscribed to pump command topic: {}", self.pump_set_topic);
        Ok(())
    }

    /// Poll the broker for pump commands until `deadline`. Returns `Ok(true)` as
    /// soon as an ON command is accepted (the switch is reset to OFF first), or
    /// `Ok(false)` when the deadline passes without one.
    pub async fn wait_for_pump_command(
        &mut self,
        pump_allowed: bool,
        deadline: Instant,
    ) -> Result<bool, Error> {
        loop {
            let Ok(event) = with_deadline(deadline, self.client.poll()).await else {
                return Ok(false); // awake window over
            };
            match event {
                Ok(Event::Publish(e)) => {
                    if self
                        .process_pump_command(
                            e.topic.as_ref().as_str(),
                            e.message.as_ref(),
                            pump_allowed,
                        )
                        .await?
                    {
                        return Ok(true);
                    }
                }
                Ok(e) => info!("Received event {:?}", e),
                Err(e) => {
                    error!("Error receiving MQTT message: {:?}", e);
                    return Err(e.into());
                }
            }
        }
    }

    /// Publish a single topic, optionally retained.
    async fn publish_str(
        &mut self,
        topic: &str,
        payload: &[u8],
        retain: bool,
    ) -> Result<(), Error> {
        let topic_ref = TopicReference::Name(TopicName::new_unchecked(
            MqttString::try_from(topic).unwrap(),
        ));
        let mut options = PublicationOptions::new(topic_ref);
        if retain {
            options = options.retain();
        }
        self.client
            .publish(&options, Bytes::Borrowed(payload))
            .await?;
        Ok(())
    }

    async fn publish_discovery_topics(&mut self) -> Result<(), Error> {
        if !DISCOVERY_MESSAGES_SENT.get() {
            info!("First run, sending discovery messages");

            for s in Sensor::iter() {
                let (discovery_topic, message) = get_sensor_discovery(&s);
                self.publish_str(&discovery_topic, message.as_bytes(), true)
                    .await?;
                info!("Discovery message sent for sensor: {}", s.name());
            }

            let (discovery_topic, message) = get_pump_switch_discovery();
            self.publish_str(&discovery_topic, message.as_bytes(), true)
                .await?;

            DISCOVERY_MESSAGES_SENT.set(true);
        } else {
            info!("Discovery messages already sent");
        }
        Ok(())
    }

    /// Returns true when an ON command was accepted and the pump should run.
    async fn process_pump_command(
        &mut self,
        topic: &str,
        data: &[u8],
        pump_allowed: bool,
    ) -> Result<bool, Error> {
        if topic != self.pump_set_topic {
            warn!("Message on unhandled topic: {}", topic);
            return Ok(false);
        }
        let Ok(message) = str::from_utf8(data) else {
            warn!("Invalid UTF-8 message on topic {}", topic);
            return Ok(false);
        };
        match message {
            "ON" => {
                // Reset the switch immediately so HA reflects the outcome,
                // and a second wake doesn't re-trigger the pump.
                self.reset_pump_switch().await?;
                if pump_allowed {
                    info!("Pump command received, starting pump");
                    Ok(true)
                } else {
                    warn!("Pump command blocked: overflow detected");
                    Ok(false)
                }
            }
            "OFF" => Ok(false), // broker echo after our own reset — ignore
            _ => {
                warn!("Unexpected payload on '{}': {}", topic, message);
                Ok(false)
            }
        }
    }

    async fn reset_pump_switch(&mut self) -> Result<(), Error> {
        let topic = self.pump_set_topic.clone();
        self.publish_str(&topic, b"OFF", true).await
    }

    async fn publish_sensor_data(&mut self, sensor_data: &SensorData) -> Result<(), Error> {
        for s in &sensor_data.data {
            let key = s.topic();
            let value = s.value();
            let message = json!({ "value": value }).to_string();
            let topic_name = format!("{DEVICE_ID}/{key}");

            info!(
                "Publishing to topic {}, message: {}",
                topic_name.as_str(),
                message.as_str()
            );

            self.publish_str(&topic_name, message.as_bytes(), false)
                .await?;
        }

        Ok(())
    }
}

fn pump_set_topic() -> String {
    format!("{DEVICE_ID}/pump/set")
}

fn get_sensor_discovery(s: &Sensor) -> (String, String) {
    let topic = s.topic();
    let mut payload = get_common_device_info(topic, s.name());
    payload["state_topic"] = json!(format!("{}/{}", DEVICE_ID, topic));
    payload["value_template"] = json!("{{ value_json.value }}");
    payload["platform"] = json!("sensor");
    payload["unique_id"] = json!(format!("{}_{}", DEVICE_ID, topic));

    let device_class = s.device_class();
    if let Some(device_class) = device_class {
        payload["device_class"] = json!(device_class);
    }

    let unit = s.unit();
    if let Some(unit) = unit {
        payload["unit_of_measurement"] = json!(unit);
        // only set state_class if unit is present - enables Home Assistant to display the unit correctly and keep track of state changes
        payload["state_class"] = json!("measurement");
        // force HA to record every incoming value even if unchanged (prevents recorder deduplication)
        payload["force_update"] = json!(true);
    }

    let discovery_topic = format!(
        "{HOMEASSISTANT_DISCOVERY_TOPIC_PREFIX}/{HOMEASSISTANT_SENSOR_TOPIC}/{DEVICE_ID}_{topic}/config"
    );

    (discovery_topic, payload.to_string())
}

fn get_pump_switch_discovery() -> (String, String) {
    let mut payload = get_common_device_info("pump", "Water pump");
    payload["command_topic"] = json!(format!("{}/pump/set", DEVICE_ID));
    payload["state_topic"] = json!(format!("{}/pump/set", DEVICE_ID));
    payload["payload_on"] = json!("ON");
    payload["payload_off"] = json!("OFF");
    payload["retain"] = json!(true);

    let discovery_topic = format!(
        "{HOMEASSISTANT_DISCOVERY_TOPIC_PREFIX}/{HOMEASSISTANT_SWITCH_TOPIC}/{DEVICE_ID}_pump/config"
    );
    (discovery_topic, payload.to_string())
}

fn get_common_device_info(topic: &str, name: &str) -> Value {
    json!({
        "name": name,
        "unique_id": format!("{}_{}", DEVICE_ID, topic),
        "device": {
            "identifiers": [DEVICE_ID],
            "name": "ESP32 Device",
            "model": "ESP32S3",
            "manufacturer": "Espressif"
        }
    })
}

#[derive(Debug)]
pub enum Error {
    Port,
    Dns(DnsError),
    DnsNoRecords,
    Connection(ConnectError),
    Broker(ReasonCode),
    Mqtt,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::Port => write!(f, "Port error"),
            Error::Dns(e) => write!(f, "DNS error: {e:?}"),
            Error::DnsNoRecords => write!(f, "DNS query returned no records"),
            Error::Connection(e) => write!(f, "Connection error: {e:?}"),
            Error::Broker(e) => write!(f, "Broker error: {e:?}"),
            Error::Mqtt => write!(f, "MQTT error"),
        }
    }
}

impl From<embassy_net::dns::Error> for Error {
    fn from(error: embassy_net::dns::Error) -> Self {
        Self::Dns(error)
    }
}

impl From<ConnectError> for Error {
    fn from(error: ConnectError) -> Self {
        Self::Connection(error)
    }
}

impl From<ParseIntError> for Error {
    fn from(_: ParseIntError) -> Self {
        Self::Port
    }
}

impl From<ReasonCode> for Error {
    fn from(error: ReasonCode) -> Self {
        Self::Broker(error)
    }
}

impl<'a> From<MqttError<'a>> for Error {
    fn from(_error: MqttError<'a>) -> Self {
        Self::Mqtt
    }
}
