use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter, State};

// Supported baudrates untuk auto-detection
const BAUDRATES: &[u32] = &[9600, 19200, 38400, 57600, 115200, 230400];
const BAUDRATE_TEST_TIMEOUT: Duration = Duration::from_millis(100);

// Global state untuk menyimpan status koneksi agar bisa dihentikan sewaktu-waktu
struct AppState {
    is_connected: Arc<AtomicBool>,
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
fn get_ports() -> Vec<String> {
    let mut port_names = Vec::new();
    if let Ok(ports) = serialport::available_ports() {
        for port in ports {
            port_names.push(port.port_name);
        }
    }
    port_names
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
