use rand::Rng;
use snap7_server::{DataStore, S7Server, ServerConfig};

#[tokio::main]
async fn main() {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(10200);

    let update_interval_ms: u64 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000);

    println!("snap7-sensor-server Starting...");
    println!("  Port: {}", port);
    println!("  Update interval: {}ms", update_interval_ms);
    println!("  Datablock:");
    println!("    DB1: Temperature (REAL, offset 0)");
    println!("    DB2: Humidity (REAL, offset 0)");
    println!("    DB3: Pressure (REAL, offset 0)");
    println!();

    let store = DataStore::new();
    store.write_bytes(1, 0, &25.0_f32.to_be_bytes());
    store.write_bytes(2, 0, &60.0_f32.to_be_bytes());
    store.write_bytes(3, 0, &101.325_f32.to_be_bytes());

    let store_for_update = store.clone();
    tokio::task::spawn_blocking(move || {
        let mut rng = rand::thread_rng();
        let mut temperature = 25.0_f32;
        let mut humidity = 60.0_f32;
        let mut pressure = 101.325_f32;

        loop {
            std::thread::sleep(std::time::Duration::from_millis(update_interval_ms));

            temperature += rng.gen_range(-0.5..0.5);
            temperature = temperature.clamp(20.0, 30.0);

            humidity += rng.gen_range(-1.0..1.0);
            humidity = humidity.clamp(40.0, 80.0);

            pressure += rng.gen_range(-0.2..0.2);
            pressure = pressure.clamp(100.0, 105.0);

            store_for_update.write_bytes(1, 0, &temperature.to_be_bytes());
            store_for_update.write_bytes(2, 0, &humidity.to_be_bytes());
            store_for_update.write_bytes(3, 0, &pressure.to_be_bytes());

            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs_f64();
            let time_str = format!(
                "{:02}:{:02}:{:02}",
                ((now % 86400.0) / 3600.0) as u32,
                ((now % 3600.0) / 60.0) as u32,
                (now % 60.0) as u32
            );
            println!(
                "[{}] Update: Temperature={:.2}°C, Humidity={:.2}%, Pressure={:.2}kPa",
                time_str, temperature, humidity, pressure
            );
        }
    });

    let addr: std::net::SocketAddr = format!("0.0.0.0:{port}").parse().unwrap();
    let cfg = ServerConfig {
        bind_addr: addr,
        max_connections: 8,
    };

    let server = S7Server::bind(cfg).await.expect("failed to bind S7Server");

    println!("S7 Server listens: 0.0.0.0:{}", port);
    println!("Press Ctrl+C to stop server");
    println!();

    tokio::select! {
        result = server.serve(store) => {
            eprintln!("Server exit: {:?}", result);
        }
        _ = tokio::signal::ctrl_c() => {
            println!("\nStopping Server...");
        }
    }
}
