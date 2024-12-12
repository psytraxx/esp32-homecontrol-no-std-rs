use defmt::{error, warn, write, Debug2Format, Format};
use embassy_net::udp::{self, UdpSocket};
use embassy_net::Stack;
use smoltcp::storage::PacketMetadata;
use sntpc::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, ToSocketAddrs};
use sntpc::{async_impl::get_time, NtpContext, NtpTimestampGenerator};

const NTP_SERVER: (u8, u8, u8, u8) = (216, 239, 35, 4);
const NTP_PORT: u16 = 123;

struct EspWifiUdpSocket<'a> {
    socket: UdpSocket<'a>,
}

impl<'a> EspWifiUdpSocket<'a> {
    fn new(socket: UdpSocket<'a>) -> Self {
        Self { socket }
    }
}

impl sntpc::async_impl::NtpUdpSocket for EspWifiUdpSocket<'_> {
    async fn send_to<T: ToSocketAddrs + Send>(&self, buf: &[u8], addr: T) -> sntpc::Result<usize> {
        let addrs = addr.to_socket_addrs().unwrap();
        for addr in addrs {
            let port = addr.port();
            let addr = match addr.ip() {
                IpAddr::V4(addr) => addr,
                _ => {
                    warn!("unssuported ip address type");
                    continue;
                }
            };
            let [a, b, c, d] = addr.octets();
            self.socket
                .send_to(
                    buf,
                    (
                        smoltcp::wire::IpAddress::from(smoltcp::wire::Ipv4Address::new(a, b, c, d)),
                        port,
                    ),
                )
                .await
                .map_err(|e| {
                    error!("error during time send: {}", e);
                    sntpc::Error::Network
                })?;
        }
        Ok(buf.len())
    }

    async fn recv_from(&self, buf: &mut [u8]) -> sntpc::Result<(usize, SocketAddr)> {
        self.socket
            .recv_from(buf)
            .await
            .map(|(bytes, meta)| {
                let smoltcp::wire::IpAddress::Ipv4(a) = meta.endpoint.addr;
                let octets = a.octets();
                (
                    bytes,
                    SocketAddr::new(
                        IpAddr::V4(Ipv4Addr::new(octets[0], octets[1], octets[2], octets[3])),
                        meta.endpoint.port,
                    ),
                )
            })
            .map_err(|e| {
                error!("error during time recv: {}", e);
                sntpc::Error::Network
            })
    }
}

impl core::fmt::Debug for EspWifiUdpSocket<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("EspWifiUdpSocket").finish()
    }
}

#[derive(Copy, Clone, Default)]
struct TimestampGen {
    duration: u64,
}

impl NtpTimestampGenerator for TimestampGen {
    fn init(&mut self) {
        self.duration = 0u64;
    }

    fn timestamp_sec(&self) -> u64 {
        self.duration >> 32
    }

    fn timestamp_subsec_micros(&self) -> u32 {
        (self.duration & 0xff_ff_ff_ffu64) as u32
    }
}

pub async fn get_unix_time(stack: Stack<'static>) -> Result<u32, Error> {
    let timestamp_gen = TimestampGen::default();
    let context = NtpContext::new(timestamp_gen);
    let server_socket_addr = SocketAddr::V4(SocketAddrV4::new(
        Ipv4Addr::new(NTP_SERVER.0, NTP_SERVER.1, NTP_SERVER.2, NTP_SERVER.3),
        NTP_PORT,
    ));
    let mut rx_meta = [PacketMetadata::EMPTY; 16];
    let mut rx_buffer = [0; 4096];
    let mut tx_meta = [PacketMetadata::EMPTY; 16];
    let mut tx_buffer = [0; 4096];
    let mut socket = embassy_net::udp::UdpSocket::new(
        stack,
        &mut rx_meta,
        &mut rx_buffer,
        &mut tx_meta,
        &mut tx_buffer,
    );
    socket.bind(9400)?;

    let socket = EspWifiUdpSocket::new(socket);

    let time = get_time(server_socket_addr, socket, context).await?;
    Ok(time.sec())
}

/// A entp error
#[derive(Debug)]
pub enum Error {
    Sntp(sntpc::Error),
    Socket(udp::BindError),
}

impl Format for Error {
    fn format(&self, fmt: defmt::Formatter) {
        match self {
            Self::Sntp(e) => write!(fmt, "NTP Error {:?}", Debug2Format(e)),
            Self::Socket(e) => write!(fmt, "Bind Error {:?}", Debug2Format(e)),
        }
    }
}

impl From<sntpc::Error> for Error {
    fn from(error: sntpc::Error) -> Self {
        Self::Sntp(error)
    }
}

impl From<udp::BindError> for Error {
    fn from(error: udp::BindError) -> Self {
        Self::Socket(error)
    }
}
