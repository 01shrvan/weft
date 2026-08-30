use std::net::TcpListener;
use std::thread;

use weft::conn::Connection;

fn main() -> std::io::Result<()> {
    let addr = match std::env::args().nth(1) {
        Some(a) => a,
        None => String::from("127.0.0.1:8080"),
    };
    let listener = TcpListener::bind(&addr)?;
    println!("weft listening on {} (h2c, prior knowledge)", addr);

    for incoming in listener.incoming() {
        let stream = match incoming {
            Ok(s) => s,
            Err(_) => continue,
        };
        thread::spawn(move || {
            let _ = stream.set_nodelay(true);
            let mut conn = Connection::new(stream);
            let _ = conn.run();
        });
    }
    Ok(())
}
