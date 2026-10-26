//! BlueZ GATT peripheral implementing [`bluelink_sync::Transport`].

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bluer::adv::Advertisement;
use bluer::gatt::local::{
    characteristic_control, Application, Characteristic, CharacteristicControl,
    CharacteristicControlEvent, CharacteristicNotify, CharacteristicNotifyMethod,
    CharacteristicRead, CharacteristicWrite, CharacteristicWriteMethod, Service,
};
use bluer::gatt::{CharacteristicReader, CharacteristicWriter};
use bluer::Uuid;
use bluelink_sync::{Transport, TransportError};
use futures::{future, pin_mut, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::fragment::{encode, FragmentDecoder, FragmentError};
use crate::info::{InfoValue, PROTOCOL_VERSION};
use crate::uuids;

/// Bound D-Bus/BlueZ setup so a stuck bluetoothd cannot hang the sync runtime forever.
const START_TIMEOUT: Duration = Duration::from_secs(15);

fn map_frag(err: FragmentError) -> TransportError {
    match err {
        FragmentError::TooLarge => TransportError::TooLarge,
        other => TransportError::Io(other.to_string()),
    }
}

fn parse_uuid(s: &str) -> Uuid {
    s.parse().expect("fixed BlueLink UUID")
}

/// Linux BLE peripheral: advertising + GATT service, framed via fragmentation.
pub struct BluezPeripheral {
    inbound: mpsc::Receiver<Vec<u8>>,
    outbound: mpsc::Sender<Vec<u8>>,
    pairing_open: Arc<AtomicBool>,
    mtu: Arc<AtomicUsize>,
    /// Cleared when the GATT link worker exits (transport becomes Closed).
    link_alive: Arc<AtomicBool>,
    worker: JoinHandle<()>,
    /// Keep advertising and GATT application alive.
    _adv: bluer::adv::AdvertisementHandle,
    _app: bluer::gatt::local::ApplicationHandle,
}

impl Drop for BluezPeripheral {
    fn drop(&mut self) {
        self.link_alive.store(false, Ordering::SeqCst);
        self.worker.abort();
    }
}

pub struct BluezPeripheralConfig {
    pub local_name: String,
    pub pairing_open: bool,
}

impl Default for BluezPeripheralConfig {
    fn default() -> Self {
        Self {
            local_name: "BlueLink".into(),
            pairing_open: false,
        }
    }
}

impl BluezPeripheral {
    pub async fn start(config: BluezPeripheralConfig) -> Result<Self, TransportError> {
        match tokio::time::timeout(START_TIMEOUT, Self::start_inner(config)).await {
            Ok(r) => r,
            Err(_) => Err(TransportError::Timeout),
        }
    }

    async fn start_inner(config: BluezPeripheralConfig) -> Result<Self, TransportError> {
        let session = bluer::Session::new()
            .await
            .map_err(|e| TransportError::Io(e.to_string()))?;
        let adapter = session
            .default_adapter()
            .await
            .map_err(|e| TransportError::Io(e.to_string()))?;
        adapter
            .set_powered(true)
            .await
            .map_err(|e| TransportError::Io(e.to_string()))?;

        let service_uuid = parse_uuid(uuids::SERVICE);
        let adv = Advertisement {
            service_uuids: std::iter::once(service_uuid).collect(),
            discoverable: Some(true),
            local_name: Some(config.local_name),
            ..Default::default()
        };
        let adv_handle = adapter
            .advertise(adv)
            .await
            .map_err(|e| TransportError::Io(e.to_string()))?;

        let pairing_open = Arc::new(AtomicBool::new(config.pairing_open));
        let pairing_for_read = pairing_open.clone();

        let (rx_control, rx_handle) = characteristic_control();
        let (tx_control, tx_handle) = characteristic_control();

        let app = Application {
            services: vec![Service {
                uuid: service_uuid,
                primary: true,
                characteristics: vec![
                    Characteristic {
                        uuid: parse_uuid(uuids::RX),
                        write: Some(CharacteristicWrite {
                            write: true,
                            write_without_response: true,
                            method: CharacteristicWriteMethod::Io,
                            ..Default::default()
                        }),
                        control_handle: rx_handle,
                        ..Default::default()
                    },
                    Characteristic {
                        uuid: parse_uuid(uuids::TX),
                        notify: Some(CharacteristicNotify {
                            notify: true,
                            method: CharacteristicNotifyMethod::Io,
                            ..Default::default()
                        }),
                        control_handle: tx_handle,
                        ..Default::default()
                    },
                    Characteristic {
                        uuid: parse_uuid(uuids::INFO),
                        read: Some(CharacteristicRead {
                            read: true,
                            fun: Box::new(move |_req| {
                                let open = pairing_for_read.clone();
                                Box::pin(async move {
                                    let info = InfoValue {
                                        protocol_version: PROTOCOL_VERSION,
                                        pairing_open: open.load(Ordering::Relaxed),
                                    };
                                    Ok(info.pack().to_vec())
                                })
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        };

        let app_handle = adapter
            .serve_gatt_application(app)
            .await
            .map_err(|e| TransportError::Io(e.to_string()))?;

        let (inbound_tx, inbound_rx) = mpsc::channel::<Vec<u8>>(16);
        let (outbound_tx, outbound_rx) = mpsc::channel::<Vec<u8>>(16);
        let mtu = Arc::new(AtomicUsize::new(23));
        let link_alive = Arc::new(AtomicBool::new(true));

        let worker = tokio::spawn(link_worker(
            rx_control,
            tx_control,
            inbound_tx,
            outbound_rx,
            mtu.clone(),
            link_alive.clone(),
        ));

        Ok(Self {
            inbound: inbound_rx,
            outbound: outbound_tx,
            pairing_open,
            mtu,
            link_alive,
            worker,
            _adv: adv_handle,
            _app: app_handle,
        })
    }

    pub fn set_pairing_open(&self, open: bool) {
        self.pairing_open.store(open, Ordering::Relaxed);
    }

    pub fn pairing_open(&self) -> bool {
        self.pairing_open.load(Ordering::Relaxed)
    }

    pub fn current_mtu(&self) -> u16 {
        self.mtu.load(Ordering::Relaxed).min(u16::MAX as usize) as u16
    }

    /// False after the GATT I/O worker has exited — further `recv` returns Closed.
    pub fn link_alive(&self) -> bool {
        self.link_alive.load(Ordering::Relaxed)
    }

    pub fn adapter_hint() -> &'static str {
        "BlueZ GATT peripheral (feature bluez)"
    }
}

async fn link_worker(
    rx_control: CharacteristicControl,
    tx_control: CharacteristicControl,
    inbound_tx: mpsc::Sender<Vec<u8>>,
    mut outbound_rx: mpsc::Receiver<Vec<u8>>,
    mtu: Arc<AtomicUsize>,
    link_alive: Arc<AtomicBool>,
) {
    pin_mut!(rx_control);
    pin_mut!(tx_control);

    let mut reader: Option<CharacteristicReader> = None;
    let mut writer: Option<CharacteristicWriter> = None;
    let mut pending: VecDeque<Vec<u8>> = VecDeque::new();
    let mut decoder = FragmentDecoder::new();
    let mut read_buf = vec![0u8; 512];

    loop {
        tokio::select! {
            biased;

            evt = rx_control.next() => {
                match evt {
                    Some(CharacteristicControlEvent::Write(req)) => {
                        let req_mtu = req.mtu();
                        mtu.store(req_mtu, Ordering::Relaxed);
                        read_buf = vec![0u8; req_mtu];
                        match req.accept() {
                            Ok(r) => reader = Some(r),
                            Err(_) => reader = None,
                        }
                        decoder.reset();
                    }
                    Some(CharacteristicControlEvent::Notify(_)) => {}
                    None => break,
                }
            }

            evt = tx_control.next() => {
                match evt {
                    Some(CharacteristicControlEvent::Notify(notifier)) => {
                        mtu.store(notifier.mtu(), Ordering::Relaxed);
                        writer = Some(notifier);
                        flush_pending(&mut pending, &mut writer, &mtu).await;
                    }
                    Some(CharacteristicControlEvent::Write(_)) => {}
                    None => break,
                }
            }

            frame = outbound_rx.recv() => {
                let Some(frame) = frame else { break };
                pending.push_back(frame);
                flush_pending(&mut pending, &mut writer, &mtu).await;
            }

            n = async {
                match reader.as_mut() {
                    Some(r) => r.read(&mut read_buf).await,
                    None => future::pending().await,
                }
            } => {
                match n {
                    Ok(0) => {
                        reader = None;
                        decoder.reset();
                    }
                    Ok(n) => {
                        let pkt = &read_buf[..n];
                        match decoder.push(pkt) {
                            Ok(Some(frame)) => {
                                if inbound_tx.send(frame).await.is_err() {
                                    break;
                                }
                            }
                            Ok(None) => {}
                            Err(_) => {
                                decoder.reset();
                            }
                        }
                    }
                    Err(_) => {
                        reader = None;
                        decoder.reset();
                    }
                }
            }
        }
    }
    link_alive.store(false, Ordering::SeqCst);
}

async fn flush_pending(
    pending: &mut VecDeque<Vec<u8>>,
    writer: &mut Option<CharacteristicWriter>,
    mtu: &AtomicUsize,
) {
    while let Some(frame) = pending.front() {
        let Some(w) = writer.as_mut() else {
            return;
        };
        let cur = mtu.load(Ordering::Relaxed).min(u16::MAX as usize) as u16;
        match encode(frame, cur) {
            Ok(packets) => {
                for pkt in packets {
                    if w.write_all(&pkt).await.is_err() {
                        *writer = None;
                        return;
                    }
                }
                pending.pop_front();
            }
            Err(_) => {
                // Invalid for current MTU — drop this frame so the queue cannot stall.
                pending.pop_front();
            }
        }
    }
}

#[async_trait]
impl Transport for BluezPeripheral {
    async fn send(&mut self, frame: &[u8]) -> Result<(), TransportError> {
        // Fail fast on size / MTU before queueing (worker waits for notify subscriber).
        encode(frame, self.current_mtu()).map_err(map_frag)?;
        self.outbound
            .send(frame.to_vec())
            .await
            .map_err(|_| TransportError::Closed)
    }

    async fn recv(&mut self) -> Result<Vec<u8>, TransportError> {
        self.inbound.recv().await.ok_or(TransportError::Closed)
    }
}
