# T210 Connect — Serial Port Temperature/Power Monitor

Desktop monitoring application for ABToolkit T210 solder station (STM32F103CB/C8T6).  
Real-time temperature and power tracking via serial communication with dual-axis charting and responsive UI.

---

## 🎯 Features

- **Real-time Monitoring**: Live temperature (`0-500°C`) and power (`0-100%`)
- **Auto Baudrate Detection**: Automatic serial baud detection (`9600` → `230400`)
- **Dual-Axis Chart**: Temperature + Power with 600-point rolling buffer
- **Responsive UI**: Optimized for floating window and fullscreen
- **Status Mapping**: Condition code mapping with color indicators
- **Windows Desktop App**: Packaged with Tauri (NSIS target)

---

## 📊 Telemetry Data Format (MCU → App)

The app reads CSV telemetry from serial stream.

### Base order (new fields appended at the end)

```csv
actualTemp,setpoint,power,status,buzzer,standbyState,standbyTimer
```

### Field definitions

| Field | Type | Normal Range | Unit | Notes |
|------|------|--------------|------|------|
| `actualTemp` | float | 0-500 | °C | Actual measured temperature |
| `setpoint` | float | 0-500 | °C | Target setpoint (shown in gauge) |
| `power` | float | 0-100 | % | Heater power / PWM duty |
| `status` | int | 0,1,2,10,11,12 | — | Device condition code |
| `buzzer` | int | 0/1 | — | Buzzer state |
| `standbyState` | int | 0/1 | — | Standby state |
| `standbyTimer` | int | 0-300 | s | Standby countdown timer |

---

## ✅ Partial Payload Compatibility

Telemetry parser is backward-compatible with older firmware payloads.

- If MCU sends fewer fields (e.g. 6 fields), app still parses available values.
- Missing trailing fields are treated as `null`.
- UI renders missing values as placeholder (`--` / `-- s`) instead of crashing.
- Chart updates only when required values (`actualTemp`, `power`) are valid numbers.

This allows schema expansion by appending new fields at the end without breaking older MCU builds.

---

## 🧭 Status Code Mapping

| Code | Condition | UI Display | Meaning |
|------|-----------|------------|---------|
| `0` | `CONDITION_SLEEP` | Sleep | Heater off / sleep mode |
| `1` | `CONDITION_STANDBY` | Standby | Waiting standby timeout |
| `2` | `CONDITION_RUN` | Running | Normal heating mode |
| `10` | `CONDITION_ERR_SENSOR` | Error: Sensor | Sensor reading invalid/fault |
| `11` | `CONDITION_ERR_OVERHEAT` | Error: Overheat | Temperature too high (`>500°C`) |
| `12` | `CONDITION_ERR_NO_HEAT` | Error: No Heat | Heater not increasing temperature |

---

## 🚀 Getting Started

### Prerequisites

- Node.js 18+
- Rust stable toolchain
- Windows environment (current package target: NSIS)

### Development

```bash
npm install
npm run tauri dev
```

### Type check

```bash
npm run check
```

### Production build

```bash
npm run tauri build
```

Output artifacts (located in `src-tauri/target/release/bundle/`):

- `nsis/T210 Connect_{version}_x64-setup.exe` — Windows installer
- `t210-connect.exe` — Standalone executable

> **Note:** `{version}` is taken from `package.json` at build time.

---

## 🔄 Version Management

The **single source of truth** for the app version is **`package.json`**.

### How to update the version

```bash
# Automatic — updates all files:
npm version patch   # 0.1.0 → 0.1.1
npm version minor   # 0.1.0 → 0.2.0
npm version major   # 0.1.0 → 1.0.0

# Or specify an exact version:
npm version 0.2.0
```

After `npm version` finishes, the **`postversion`** hook automatically runs `node scripts/sync-version.js`, which syncs the version to:

| File | Role |
|------|------|
| `package.json` | Single source of truth |
| `src-tauri/tauri.conf.json` | Tauri app config (runtime version) |
| `src-tauri/Cargo.toml` | Rust crate version |

### Manual sync

```bash
npm run version:sync
```

---

## 📁 Project Structure

```
T210 Connect/
├── package.json              # Node deps + version source of truth
├── scripts/
│   └── sync-version.js       # Version sync script
├── src/                      # SvelteKit frontend
│   ├── app.html              # HTML shell
│   ├── global.d.ts           # Global type declarations
│   ├── lib/                  # Shared components / assets
│   └── routes/
│       └── +page.svelte      # Main UI (single-page)
├── src-tauri/                # Tauri / Rust backend
│   ├── Cargo.toml            # Rust deps + version
│   ├── tauri.conf.json       # Tauri config + version
│   └── src/
│       └── lib.rs            # Serial port logic + Tauri commands
├── static/                   # Static assets (icons, fonts)
└── LICENSE
```

---

## 🖼️ Icon Assets (Windows)

Current app icon files are stored in:

- `src-tauri/icons/32x32.png`
- `src-tauri/icons/128x128.png`
- `src-tauri/icons/256x256.png`
- `src-tauri/icons/favicon.ico`

---

## 🔐 License

This project is licensed under the **MIT License**.

See `LICENSE` for full legal terms.

---

## 📚 Tech Stack

- SvelteKit 5 + TypeScript + Vite
- Tauri 2 (Rust backend)
- `serialport` crate
- `uPlot` charting