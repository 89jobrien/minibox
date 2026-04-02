//! Bridge network adapter — Linux-only.
#![cfg(target_os = "linux")]

use ipnet::IpNet;
use std::collections::BTreeSet;
use std::net::IpAddr;

/// Sequential IP allocator within a subnet.
///
/// Skips the network address (`.0`) and gateway address (`.1`).
/// Released IPs are returned to the pool.
pub struct IpAllocator {
    subnet:    IpNet,
    available: BTreeSet<u32>,  // IPv4 host parts only
    gateway:   u32,
}

impl IpAllocator {
    pub fn new(subnet: IpNet) -> Self {
        let base = match subnet.network() {
            IpAddr::V4(a) => u32::from(a),
            IpAddr::V6(_) => panic!("IPv6 not supported in IpAllocator"),
        };
        let hosts = subnet.hosts().filter_map(|ip| {
            if let IpAddr::V4(a) = ip { Some(u32::from(a)) } else { None }
        });
        let mut available: BTreeSet<u32> = hosts.collect();
        let gateway = base + 1;
        available.remove(&gateway);  // reserve gateway
        Self { subnet, available, gateway }
    }

    pub fn allocate(&mut self) -> Option<IpAddr> {
        self.available.pop_first().map(|n| IpAddr::V4(n.into()))
    }

    pub fn release(&mut self, ip: IpAddr) {
        if let IpAddr::V4(a) = ip {
            let n = u32::from(a);
            if self.subnet.contains(&ip) && n != self.gateway {
                self.available.insert(n);
            }
        }
    }

    pub fn gateway(&self) -> IpAddr {
        IpAddr::V4(self.gateway.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    #[test]
    fn test_ip_allocator_skips_network_and_gateway() {
        let subnet: ipnet::IpNet = "172.20.0.0/16".parse().unwrap();
        let mut alloc = IpAllocator::new(subnet);

        let first = alloc.allocate().unwrap();
        // Must not be .0 (network) or .1 (gateway)
        assert_ne!(first, "172.20.0.0".parse::<IpAddr>().unwrap());
        assert_ne!(first, "172.20.0.1".parse::<IpAddr>().unwrap());
        assert_eq!(first, "172.20.0.2".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn test_ip_allocator_release_and_reuse() {
        let subnet: ipnet::IpNet = "172.20.0.0/16".parse().unwrap();
        let mut alloc = IpAllocator::new(subnet);

        let ip1 = alloc.allocate().unwrap();
        alloc.release(ip1);
        let ip2 = alloc.allocate().unwrap();
        assert_eq!(ip1, ip2); // released IP is reused
    }
}
