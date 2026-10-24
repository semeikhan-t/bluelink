//! Spike: advertise BlueLink GATT so nRF Connect can Read info / Write rx / Subscribe tx.
//!
//! ```bash
//! cargo run -p bluelink_ble --example peripheral --features bluez
//! ```

use bluelink_ble::{BluezPeripheral, BluezPeripheralConfig, InfoValue, PROTOCOL_VERSION, INFO, RX, SERVICE, TX};
use bluelink_sync::Transport;
use tokio::signal;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut peripheral = BluezPeripheral::start(BluezPeripheralConfig {
        local_name: "BlueLink".into(),
        pairing_open: true,
    })
    .await?;

    println!("BlueLink peripheral up");
    println!("  service {SERVICE}");
    println!("  rx      {RX}  (Write Without Response)");
    println!("  tx      {TX}  (Notify)");
    println!("  info    {INFO} (Read)");
    let info = InfoValue {
        protocol_version: PROTOCOL_VERSION,
        pairing_open: true,
    };
    println!(
        "  info bytes: {:02x?}  (protocol={}, pairing_open={})",
        info.pack(),
        PROTOCOL_VERSION,
        peripheral.pairing_open()
    );
    println!("Subscribe to tx, write to rx — frames are echoed. Ctrl+C to stop.");

    loop {
        tokio::select! {
            _ = signal::ctrl_c() => {
                println!("shutting down");
                break;
            }
            frame = peripheral.recv() => {
                match frame {
                    Ok(data) => {
                        println!("recv {} bytes (mtu≈{})", data.len(), peripheral.current_mtu());
                        if let Err(e) = peripheral.send(&data).await {
                            eprintln!("echo send: {e}");
                        } else {
                            println!("echoed");
                        }
                    }
                    Err(e) => {
                        eprintln!("recv ended: {e}");
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}
