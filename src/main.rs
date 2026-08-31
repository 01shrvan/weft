use std::io::Read;
use std::net::{Shutdown, TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

use weft::conn::Connection;

fn serve(stream: TcpStream) {
    let _ = stream.set_nodelay(true);
    let worker = match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut conn = Connection::new(worker);
    let _ = conn.run();

    let mut tail = stream;
    let _ = tail.shutdown(Shutdown::Write);
    let _ = tail.set_read_timeout(Some(Duration::from_millis(250)));
    let mut sink = [0u8; 2048];
    loop {
        match tail.read(&mut sink) {
            Ok(0) => break,
            Ok(_) => continue,
            Err(_) => break,
        }
    }
}

fn main() -> std::io::Result<()> {
    let addr = match std::env::args().nth(1) {
        Some(a) => a,
        None => String::from("127.0.0.1:8080"),
    };
    let listener = TcpListener::bind(&addr)?;
    println!("weft listening on {} (h2c, prior knowledge)", addr);

    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                thread::spawn(move || serve(stream));
            }
            Err(_) => continue,
        }
    }
    Ok(())
}
