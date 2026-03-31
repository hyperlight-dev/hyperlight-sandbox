//! Socket trait implementations (stubs — all return NotSupported).
#![allow(unused_variables)]

use hyperlight_common::resource::BorrowedResourceGuard;
use hyperlight_host::HyperlightError;
use wasi::clocks::monotonic_clock;
use wasi::sockets::{ip_name_lookup, network, tcp, udp};

use crate::HostState;
use crate::bindings::wasi;
use crate::wasi_impl::resource::Resource;
use crate::wasi_impl::types::pollable::AnyPollable;
use crate::wasi_impl::types::stream::Stream;

type HlResult<T> = Result<T, HyperlightError>;

// ---------------------------------------------------------------------------
// Sockets: Network
// ---------------------------------------------------------------------------

impl network::Network for HostState {
    type T = u32;
}

impl wasi::sockets::Network for HostState {}

impl wasi::sockets::InstanceNetwork<u32> for HostState {
    fn instance_network(&mut self) -> HlResult<u32> {
        Ok(0)
    }
}

// ---------------------------------------------------------------------------
// Sockets: TCP
// ---------------------------------------------------------------------------

impl
    tcp::TcpSocket<
        monotonic_clock::Duration,
        network::ErrorCode,
        Resource<Stream>,
        network::IpAddressFamily,
        network::IpSocketAddress,
        u32,
        Resource<Stream>,
        Resource<AnyPollable>,
    > for HostState
{
    type T = u32;
    fn start_bind(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
        network: BorrowedResourceGuard<u32>,
        local_address: network::IpSocketAddress,
    ) -> HlResult<Result<(), network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn finish_bind(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
    ) -> HlResult<Result<(), network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn start_connect(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
        network: BorrowedResourceGuard<u32>,
        remote_address: network::IpSocketAddress,
    ) -> HlResult<Result<(), network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn finish_connect(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
    ) -> HlResult<Result<(Resource<Stream>, Resource<Stream>), network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn start_listen(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
    ) -> HlResult<Result<(), network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn finish_listen(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
    ) -> HlResult<Result<(), network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn accept(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
    ) -> HlResult<Result<(u32, Resource<Stream>, Resource<Stream>), network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn local_address(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
    ) -> HlResult<Result<network::IpSocketAddress, network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn remote_address(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
    ) -> HlResult<Result<network::IpSocketAddress, network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn is_listening(&mut self, self_: BorrowedResourceGuard<u32>) -> HlResult<bool> {
        Ok(false)
    }
    fn address_family(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
    ) -> HlResult<network::IpAddressFamily> {
        Ok(network::IpAddressFamily::Ipv4)
    }
    fn set_listen_backlog_size(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
        value: u64,
    ) -> HlResult<Result<(), network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn keep_alive_enabled(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
    ) -> HlResult<Result<bool, network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn set_keep_alive_enabled(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
        value: bool,
    ) -> HlResult<Result<(), network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn keep_alive_idle_time(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
    ) -> HlResult<Result<monotonic_clock::Duration, network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn set_keep_alive_idle_time(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
        value: monotonic_clock::Duration,
    ) -> HlResult<Result<(), network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn keep_alive_interval(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
    ) -> HlResult<Result<monotonic_clock::Duration, network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn set_keep_alive_interval(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
        value: monotonic_clock::Duration,
    ) -> HlResult<Result<(), network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn keep_alive_count(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
    ) -> HlResult<Result<u32, network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn set_keep_alive_count(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
        value: u32,
    ) -> HlResult<Result<(), network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn hop_limit(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
    ) -> HlResult<Result<u8, network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn set_hop_limit(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
        value: u8,
    ) -> HlResult<Result<(), network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn receive_buffer_size(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
    ) -> HlResult<Result<u64, network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn set_receive_buffer_size(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
        value: u64,
    ) -> HlResult<Result<(), network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn send_buffer_size(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
    ) -> HlResult<Result<u64, network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn set_send_buffer_size(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
        value: u64,
    ) -> HlResult<Result<(), network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn subscribe(&mut self, self_: BorrowedResourceGuard<u32>) -> HlResult<Resource<AnyPollable>> {
        Ok(Resource::new(AnyPollable::future(std::future::ready(()))))
    }
    fn shutdown(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
        shutdown_type: tcp::ShutdownType,
    ) -> HlResult<Result<(), network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
}

impl
    wasi::sockets::Tcp<
        monotonic_clock::Duration,
        network::ErrorCode,
        Resource<Stream>,
        network::IpAddressFamily,
        network::IpSocketAddress,
        u32,
        Resource<Stream>,
        Resource<AnyPollable>,
    > for HostState
{
}

impl wasi::sockets::TcpCreateSocket<network::ErrorCode, network::IpAddressFamily, u32>
    for HostState
{
    fn create_tcp_socket(
        &mut self,
        address_family: network::IpAddressFamily,
    ) -> HlResult<Result<u32, network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
}

// ---------------------------------------------------------------------------
// Sockets: UDP
// ---------------------------------------------------------------------------

impl
    udp::IncomingDatagramStream<network::ErrorCode, network::IpSocketAddress, Resource<AnyPollable>>
    for HostState
{
    type T = u32;
    fn receive(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
        max_results: u64,
    ) -> HlResult<Result<Vec<udp::IncomingDatagram<network::IpSocketAddress>>, network::ErrorCode>>
    {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn subscribe(&mut self, self_: BorrowedResourceGuard<u32>) -> HlResult<Resource<AnyPollable>> {
        Ok(Resource::new(AnyPollable::future(std::future::ready(()))))
    }
}

impl
    udp::OutgoingDatagramStream<network::ErrorCode, network::IpSocketAddress, Resource<AnyPollable>>
    for HostState
{
    type T = u32;
    fn check_send(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
    ) -> HlResult<Result<u64, network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn send(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
        datagrams: Vec<udp::OutgoingDatagram<network::IpSocketAddress>>,
    ) -> HlResult<Result<u64, network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn subscribe(&mut self, self_: BorrowedResourceGuard<u32>) -> HlResult<Resource<AnyPollable>> {
        Ok(Resource::new(AnyPollable::future(std::future::ready(()))))
    }
}

impl
    udp::UdpSocket<
        network::ErrorCode,
        u32,
        network::IpAddressFamily,
        network::IpSocketAddress,
        u32,
        u32,
        Resource<AnyPollable>,
    > for HostState
{
    type T = u32;
    fn start_bind(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
        network: BorrowedResourceGuard<u32>,
        local_address: network::IpSocketAddress,
    ) -> HlResult<Result<(), network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn finish_bind(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
    ) -> HlResult<Result<(), network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn stream(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
        remote_address: Option<network::IpSocketAddress>,
    ) -> HlResult<Result<(u32, u32), network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn local_address(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
    ) -> HlResult<Result<network::IpSocketAddress, network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn remote_address(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
    ) -> HlResult<Result<network::IpSocketAddress, network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn address_family(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
    ) -> HlResult<network::IpAddressFamily> {
        Ok(network::IpAddressFamily::Ipv4)
    }
    fn unicast_hop_limit(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
    ) -> HlResult<Result<u8, network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn set_unicast_hop_limit(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
        value: u8,
    ) -> HlResult<Result<(), network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn receive_buffer_size(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
    ) -> HlResult<Result<u64, network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn set_receive_buffer_size(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
        value: u64,
    ) -> HlResult<Result<(), network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn send_buffer_size(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
    ) -> HlResult<Result<u64, network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn set_send_buffer_size(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
        value: u64,
    ) -> HlResult<Result<(), network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
    fn subscribe(&mut self, self_: BorrowedResourceGuard<u32>) -> HlResult<Resource<AnyPollable>> {
        Ok(Resource::new(AnyPollable::future(std::future::ready(()))))
    }
}

impl
    wasi::sockets::Udp<
        network::ErrorCode,
        network::IpAddressFamily,
        network::IpSocketAddress,
        u32,
        Resource<AnyPollable>,
    > for HostState
{
}

impl wasi::sockets::UdpCreateSocket<network::ErrorCode, network::IpAddressFamily, u32>
    for HostState
{
    fn create_udp_socket(
        &mut self,
        address_family: network::IpAddressFamily,
    ) -> HlResult<Result<u32, network::ErrorCode>> {
        Ok(Err(network::ErrorCode::NotSupported))
    }
}

// ---------------------------------------------------------------------------
// Sockets: IP Name Lookup
// ---------------------------------------------------------------------------

impl
    ip_name_lookup::ResolveAddressStream<
        network::ErrorCode,
        network::IpAddress,
        Resource<AnyPollable>,
    > for HostState
{
    type T = u32;
    fn resolve_next_address(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
    ) -> HlResult<Result<Option<network::IpAddress>, network::ErrorCode>> {
        Ok(Ok(None))
    }
    fn subscribe(&mut self, self_: BorrowedResourceGuard<u32>) -> HlResult<Resource<AnyPollable>> {
        Ok(Resource::new(AnyPollable::future(std::future::ready(()))))
    }
}

impl wasi::sockets::IpNameLookup<network::ErrorCode, network::IpAddress, u32, Resource<AnyPollable>>
    for HostState
{
    fn resolve_addresses(
        &mut self,
        network: BorrowedResourceGuard<u32>,
        name: String,
    ) -> HlResult<Result<u32, network::ErrorCode>> {
        Ok(Err(network::ErrorCode::PermanentResolverFailure))
    }
}
