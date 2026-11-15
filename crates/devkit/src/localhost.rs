use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::thread;
use std::time::Duration;

pub const DEV_PORTS: [u16; 12] = [3000, 3001, 4000, 4200, 5000, 5173, 5174, 8000, 8080, 8081, 8888, 9000];

const TIMEOUT: Duration = Duration::from_millis(300);

pub fn open_dev_ports() -> Vec<u16> {
    probe(&DEV_PORTS, TIMEOUT)
}

fn probe(ports: &[u16], timeout: Duration) -> Vec<u16> {
    thread::scope(|s| {
        let handles: Vec<_> = ports
            .iter()
            .map(|&port| {
                s.spawn(move || {
                    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
                    TcpStream::connect_timeout(&addr, timeout).is_ok().then_some(port)
                })
            })
            .collect();
        handles.into_iter().filter_map(|h| h.join().ok().flatten()).collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn finds_listening_port_only() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let open = listener.local_addr().unwrap().port();
        let closed = {
            let l = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            l.local_addr().unwrap().port()
        };
        assert_eq!(probe(&[closed, open], TIMEOUT), vec![open]);
    }
}
