use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

// Supported baudrates untuk auto-detection
const BAUDRATES: &[u32] = &[9600, 19200, 38400, 57600, 115200, 230400];
const BAUDRATE_TEST_TIMEOUT: Duration = Duration::from_millis(100);

// Global state untuk menyimpan status koneksi agar bisa dihentikan sewaktu-waktu
struct AppState {
    is_connected: Arc<AtomicBool>,
}

#[derive(Serialize)]
struct PortInfo {
    port_name: String,
    display_name: String,
    port_type: String,
    vid: Option<u16>,
    pid: Option<u16>,
    serial_number: Option<String>,
    manufacturer: Option<String>,
    product: Option<String>,
}

fn format_usb_display_name(
    port_name: &str,
    manufacturer: &Option<String>,
    product: &Option<String>,
) -> String {
    let raw_device_name = match (product.as_deref(), manufacturer.as_deref()) {
        (Some(product), _) if !product.trim().is_empty() => product.trim().to_string(),
        (_, Some(manufacturer)) if !manufacturer.trim().is_empty() => manufacturer.trim().to_string(),
        (None, None) => "USB Serial Device".to_string(),
        _ => "USB Serial Device".to_string(),
    };

    let compact_device_name = raw_device_name
        .replace("STMicroelectronics", "STM")
        .replace("Virtual COM Port", "VCP")
        .replace("USB-SERIAL", "USB Serial")
        .replace("USB Serial Device", "USB Serial")
        .replace("USB2.0-Serial", "USB Serial")
        .replace("USB to UART", "USB UART")
        .replace("Silicon Labs", "SiLabs")
        .replace("Communications", "Comms");

    let short_device_name = if compact_device_name.to_ascii_uppercase().contains("CH340") {
        "CH340".to_string()
    } else if compact_device_name.to_ascii_uppercase().contains("CH341") {
        "CH341".to_string()
    } else if compact_device_name.len() > 22 {
        format!("{}...", compact_device_name.chars().take(22).collect::<String>())
    } else {
        compact_device_name
    };

    format!("{} — {}", port_name, short_device_name)
}

/// Coba deteksi baudrate dengan mengecek apakah ada valid data stream
fn try_detect_baudrate(port_name: &str) -> Option<u32> {
    for &baudrate in BAUDRATES {
        if let Ok(mut port) = serialport::new(port_name, baudrate)
            .timeout(BAUDRATE_TEST_TIMEOUT)
            .open()
        {
            let _ = port.write_data_terminal_ready(true);
            
            let mut test_buf = vec![0u8; 256];

            // Test: coba baca data dalam waktu singkat
            for _ in 0..5 {
                match port.read(test_buf.as_mut_slice()) {
                    Ok(n) if n > 0 => {
                        // Check jika data terlihat valid (printable atau CSV-like)
                        let data_str = String::from_utf8_lossy(&test_buf[..n]);
                        if data_str.contains(',') || data_str.chars().all(|c| c.is_ascii_digit() || c.is_ascii_punctuation() || c.is_whitespace()) {
                            eprintln!("Baudrate detected: {} bps", baudrate);
                            return Some(baudrate);
                        }
                    }
                    Ok(_) => {
                        // Data read tapi ukuran 0, continue testing
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {
                        // Timeout OK, lanjut test
                    }
                    Err(_) => break,
                }
                thread::sleep(Duration::from_millis(20));
            }

            eprintln!("Baudrate {} failed - no valid data", baudrate);
        }
    }

    eprintln!("Could not auto-detect baudrate, defaulting to 115200");
    None
}

#[tauri::command]
fn get_ports() -> Vec<PortInfo> {
    let mut port_infos = Vec::new();
    if let Ok(ports) = serialport::available_ports() {
        for port in ports {
            let port_name = port.port_name;

            let port_info = match port.port_type {
                serialport::SerialPortType::UsbPort(usb_info) => {
                    let display_name = format_usb_display_name(
                        &port_name,
                        &usb_info.manufacturer,
                        &usb_info.product,
                    );

                    PortInfo {
                        port_name,
                        display_name,
                        port_type: "USB".to_string(),
                        vid: Some(usb_info.vid),
                        pid: Some(usb_info.pid),
                        serial_number: usb_info.serial_number,
                        manufacturer: usb_info.manufacturer,
                        product: usb_info.product,
                    }
                }
                serialport::SerialPortType::BluetoothPort => PortInfo {
                    display_name: format!("{} — Bluetooth", port_name),
                    port_name,
                    port_type: "Bluetooth".to_string(),
                    vid: None,
                    pid: None,
                    serial_number: None,
                    manufacturer: None,
                    product: None,
                },
                serialport::SerialPortType::PciPort => PortInfo {
                    display_name: format!("{} — PCI", port_name),
                    port_name,
                    port_type: "PCI".to_string(),
                    vid: None,
                    pid: None,
                    serial_number: None,
                    manufacturer: None,
                    product: None,
                },
                serialport::SerialPortType::Unknown => PortInfo {
                    display_name: format!("{} — Serial", port_name),
                    port_name,
                    port_type: "Unknown".to_string(),
                    vid: None,
                    pid: None,
                    serial_number: None,
                    manufacturer: None,
                    product: None,
                },
            };

            port_infos.push(port_info);
        }
    }
    port_infos
}

#[tauri::command]
fn connect_port(port_name: String, app_handle: AppHandle, state: State<'_, AppState>) -> Result<String, String> {
    if state
        .is_connected
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("Port masih terhubung. Disconnect dulu sebelum connect ulang.".to_string());
    }

    // Try auto-detect baudrate first, fallback to 115200
    let baudrate = try_detect_baudrate(&port_name).unwrap_or(115200);
    eprintln!("Connecting to {} with baudrate: {}", port_name, baudrate);

    let port = serialport::new(&port_name, baudrate)
        .timeout(Duration::from_millis(50))
        .open();

    match port {
        Ok(mut serial) => {
            // --- TAMBAHAN KRUSIAL: AKTIFKAN DTR ---
            // Memberi tahu STM32 bahwa PC sudah siap menerima data
            let _ = serial.write_data_terminal_ready(true);
            
            let is_connected = state.is_connected.clone();
            let app_handle_clone = app_handle.clone();
            let port_name_clone = port_name.clone();

            // Spawn background thread agar UI tidak nge-lag
            thread::spawn(move || {
                let mut serial_buf: Vec<u8> = vec![0; 1024];
                let mut string_buffer = String::new();
                const MAX_BUFFER_SIZE: usize = 65536; // 64KB max buffer

                while is_connected.load(Ordering::SeqCst) {
                    match serial.read(serial_buf.as_mut_slice()) {
                        Ok(t) if t > 0 => {
                            let s = String::from_utf8_lossy(&serial_buf[..t]);
                            string_buffer.push_str(&s);

                            // Prevent buffer overflow from garbled/incomplete data
                            if string_buffer.len() > MAX_BUFFER_SIZE {
                                eprintln!("Serial buffer overflow detected, clearing buffer");
                                string_buffer.clear();
                            }

                            while let Some(i) = string_buffer.find('\n') {
                                let line = string_buffer[..i].trim().to_string();
                                string_buffer.drain(..=i);
                                
                                if !line.is_empty() {
                                    let _ = app_handle_clone.emit("serial-data", line);
                                }
                            }
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => (),
                        Err(e) => {
                            is_connected.store(false, Ordering::SeqCst);
                            let _ = app_handle_clone.emit(
                                "serial-disconnected",
                                format!("Port {} terputus: {}", port_name_clone, e),
                            );
                            break;
                        }
                        _ => {}
                    }
                }
            });
            Ok(format!("Terkoneksi ke {} ({}bps)", port_name, baudrate))
        }
        Err(e) => {
            state.is_connected.store(false, Ordering::SeqCst);
            Err(format!("Gagal membuka {} ({}bps): {}", port_name, baudrate, e))
        }
    }
}

#[tauri::command]
fn disconnect_port(app_handle: AppHandle, state: State<'_, AppState>) -> String {
    // Set flag ke false, thread pembaca serial akan otomatis berhenti
    state.is_connected.store(false, Ordering::SeqCst);
    // Grace period untuk thread pembaca sempat exit dengan clean
    thread::sleep(Duration::from_millis(50));
    let _ = app_handle.emit("serial-disconnected", "Koneksi diputus manual");
    "Koneksi diputus".to_string()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        // Daftarkan global state ke dalam aplikasi
        .manage(AppState {
            is_connected: Arc::new(AtomicBool::new(false)),
        })
        // Daftarkan ke-3 fungsi command
        .invoke_handler(tauri::generate_handler![get_ports, connect_port, disconnect_port])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
