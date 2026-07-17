<script lang="ts">
  import { getVersion } from "@tauri-apps/api/app";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { onDestroy, onMount } from "svelte";
  import uPlot from "uplot";
  import "uplot/dist/uPlot.min.css";
  import refreshIcon from "$lib/icons/solar--refresh-bold.svg";

  type Telemetry = {
    actualTemp: number;
    setpoint: number;
    power: number;
    status: number;
    buzzer: number;
    standbyState: number;
    standbyTimer: number;
  };

  type PortInfo = {
    port_name: string;
    display_name: string;
    port_type: string;
    vid: number | null;
    pid: number | null;
    serial_number: string | null;
    manufacturer: string | null;
    product: string | null;
  };

  type Theme = "dark" | "light";

  let ports: PortInfo[] = [];
  let selectedPort = "";
  let isConnected = false;
  let isConnectionBusy = false;
  let connectionStatus = "Disconnected";
  let showAbout = false;
  let appVersion = "";
  let theme: Theme = "dark";

  let actualTemp: number | null = null;
  let setpoint: number | null = null;
  let power: number | null = null;
  let status: number | null = null;
  let buzzer: number | null = null;
  let standbyState: number | null = null;
  let standbyTimer: number | null = null;
  let serialBuffer = "";
  let lastTelemetryAt = 0;
  let isTelemetryStale = false;
  let staleTimer: ReturnType<typeof setInterval> | null = null;
  let chartReady = false;
  let chartQueue: Array<{ actualTemp: number; power: number }> = [];

  const STATUS_CONDITIONS: {
    [key: number]: { text: string; label: string; color: string };
  } = {
    0: {
      text: "Sleep",
      label: "Dalam mode tidur (heater mati)",
      color: "color-gray"
    },
    1: {
      text: "Standby",
      label: "Mode standby (menunggu timeout)",
      color: "color-yellow"
    },
    2: {
      text: "Running",
      label: "Mode normal/aktif (heating)",
      color: "color-green"
    },
    10: {
      text: "Error: Sensor",
      label: "Error: sensor pembacaan rusak/salah",
      color: "color-red"
    },
    11: {
      text: "Error: Overheat",
      label: "Error: suhu terlalu tinggi (>500°C)",
      color: "color-red"
    },
    12: {
      text: "Error: No Heat",
      label: "Error: heater tidak bekerja (suhu tidak naik)",
      color: "color-red"
    }
  };

  function formatNumber(value: number | null, digits = 0): string {
    return value === null ? "--" : value.toFixed(digits);
  }

  function clamp(value: number, min: number, max: number): number {
    return Math.min(Math.max(value, min), max);
  }

  function getStatusInfo(statusCode: number) {
    return (
      STATUS_CONDITIONS[statusCode] || {
        text: `UNKNOWN_${statusCode}`,
        label: `Status tidak dikenali (${statusCode})`,
        color: "color-gray"
      }
    );
  }

  $: statusInfo = getStatusInfo(status ?? -1);
  $: buzzerText = buzzer === null ? "--" : buzzer === 1 ? "ON" : "OFF";
  $: standbyText = standbyState === null ? "--" : standbyState === 1 ? "Active" : "Inactive";
  $: connectionDisplay = isTelemetryStale ? `${connectionStatus} (No data > 3s)` : connectionStatus;

  let unlistenSerialData: (() => void) | null = null;
  let unlistenSerialDisconnected: (() => void) | null = null;

  let chartTarget: HTMLDivElement | null = null;
  let plot: uPlot | null = null;
  let resizeHandler: (() => void) | null = null;
  let resizeObserver: ResizeObserver | null = null;
  const windowSec = 60;
  const maxPoints = windowSec * 2;
  const tData: number[] = [];
  const actualData: number[] = [];
  const powerData: number[] = [];
  let startedAt = 0;
  let lastChartPointAt = 0;
  let chartFrameId: number | null = null;
  let chartNeedsRedraw = false;
  const minChartPointIntervalMs = 150;
  const temperatureTicks = [0, 100, 200, 300, 400, 500];
  const powerTicks = [0, 20, 40, 60, 80, 100];

  function scheduleChartRedraw() {
    if (!plot || chartFrameId !== null) return;

    chartFrameId = requestAnimationFrame(() => {
      chartFrameId = null;

      if (!plot || !chartNeedsRedraw) return;

      chartNeedsRedraw = false;
      plot.setData([tData, actualData, powerData]);
    });
  }

  function syncPlotSize() {
    if (!plot || !chartTarget) return;
    plot.setSize({
      width: Math.max(chartTarget.clientWidth, 270),
      height: Math.max(chartTarget.clientHeight, 150)
    });
  }

  function parseTelemetryLine(line: string): Telemetry | null {
    const trimmedLine = line.trim();
    if (!trimmedLine) return null;
    if (trimmedLine.startsWith("actualTemp,")) return null;

    const cols = trimmedLine.split(",");
    if (cols.length !== 7) return null;

    const nums = cols.map((value) => Number(value.trim()));
    if (nums.some((value) => !Number.isFinite(value))) return null;

    return {
      actualTemp: nums[0],
      setpoint: nums[1],
      power: nums[2],
      status: nums[3],
      buzzer: nums[4],
      standbyState: nums[5],
      standbyTimer: nums[6]
    };
  }

  function updateTelemetryUi(telemetry: Telemetry) {
    lastTelemetryAt = Date.now();
    isTelemetryStale = false;

    actualTemp = clamp(telemetry.actualTemp, 0, 550);
    setpoint = telemetry.setpoint;
    power = clamp(telemetry.power, 0, 100);
    status = telemetry.status;
    buzzer = telemetry.buzzer;
    standbyState = telemetry.standbyState;
    standbyTimer = Math.max(0, telemetry.standbyTimer);

    pushChartPoint({ actualTemp, power });
  }

  function onSerialChunk(text: string) {
    serialBuffer += text;

    const lines = serialBuffer.split("\n");
    serialBuffer = lines.pop() ?? "";

    for (const rawLine of lines) {
      const telemetry = parseTelemetryLine(rawLine);
      if (!telemetry) continue;

      updateTelemetryUi(telemetry);
    }
  }

  function onSerialData(payload: string) {
    if (payload.includes("\n")) {
      onSerialChunk(payload);
      return;
    }

    const telemetry = parseTelemetryLine(payload);
    if (!telemetry) return;

    updateTelemetryUi(telemetry);
  }

  function updateStaleState() {
    isTelemetryStale = isConnected && lastTelemetryAt > 0 && Date.now() - lastTelemetryAt > 3000;
  }

  function applyTheme(nextTheme: Theme) {
    theme = nextTheme;
    document.body.classList.toggle("theme-light", theme === "light");
    localStorage.setItem("t210-theme", theme);
  }

  function toggleTheme() {
    applyTheme(theme === "dark" ? "light" : "dark");
  }

  async function openWebsite() {
    try {
      await openUrl("https://abtoolkit.id/");
    } catch (error) {
      console.error("Failed to open website:", error);
    }
  }

  let chartCounter = 0;

  function pushChartPoint(sample: { actualTemp: number; power: number }) {
    if (!chartReady) {
      chartQueue.push(sample);
      return;
    }

    if (!plot) return;

    const now = performance.now();

    if (startedAt === 0) {
      startedAt = now;
    }

    const shouldAppendPoint = tData.length === 0 || now - lastChartPointAt >= minChartPointIntervalMs;

    if (!shouldAppendPoint) {
      actualData[actualData.length - 1] = sample.actualTemp;
      powerData[powerData.length - 1] = sample.power;
      chartNeedsRedraw = true;
      scheduleChartRedraw();
      return;
    }

    lastChartPointAt = now;

    const elapsedSec = (now - startedAt) / 1000;

    tData.push(elapsedSec);
    actualData.push(sample.actualTemp);
    powerData.push(sample.power);

    if (tData.length > maxPoints) {
      tData.shift();
      actualData.shift();
      powerData.shift();
    }

    chartNeedsRedraw = true;
    scheduleChartRedraw();
  }

  function initPlot() {
    if (!chartTarget) return;

    const width = Math.max(chartTarget.clientWidth, 270);
    const height = Math.max(chartTarget.clientHeight, 150);

    plot = new uPlot(
      {
        width,
        height,
        legend: { show: false },
        cursor: {
          show: false
        },
        scales: {
          x: {
            time: false,
            range: (_u, _min, max) => [Math.max(0, max - windowSec), Math.max(windowSec, max)]
          },
          temp: { auto: false, range: [0, 500] },
          power: { auto: false, range: [0, 100] }
        },
        series: [
          { label: "Time" },
          { label: "Actual", scale: "temp", stroke: "#fb923c", width: 2 },
          { label: "Power", scale: "power", stroke: "#4ade80", width: 2 }
        ],
        axes: [
          {
            stroke: "#64748b",
            grid: { stroke: "#1f2937" },
            values: (_u, vals) => vals.map((v) => `${v.toFixed(0)}s`)
          },
          {
            scale: "temp",
            stroke: "#94a3b8",
            grid: { stroke: "#1f2937" },
            incrs: [100],
            splits: () => temperatureTicks,
            values: (_u, vals) => vals.map((v) => v.toFixed(0))
          },
          {
            side: 1,
            scale: "power",
            stroke: "#94a3b8",
            grid: { stroke: "#1f2937" },
            incrs: [20],
            splits: () => powerTicks,
            values: (_u, vals) => vals.map((v) => v.toFixed(0))
          }
        ]
      },
      [tData, actualData, powerData],
      chartTarget
    );

    resizeHandler = () => {
      syncPlotSize();
    };
    window.addEventListener("resize", resizeHandler);

    resizeObserver = new ResizeObserver(() => {
      syncPlotSize();
    });
    resizeObserver.observe(chartTarget);

    chartReady = true;
    while (chartQueue.length > 0) {
      const sample = chartQueue.shift();
      if (sample) pushChartPoint(sample);
    }
  }

  async function scanPorts() {
    ports = await invoke<PortInfo[]>("get_ports");
    if (ports.length > 0 && !selectedPort) {
      selectedPort = ports[0].port_name;
    }

    if (selectedPort && !ports.some((port) => port.port_name === selectedPort)) {
      selectedPort = ports[0]?.port_name ?? "";
    }
  }

  async function connectSerial() {
    if (!selectedPort) {
      connectionStatus = "Port belum dipilih";
      return;
    }

    connectionStatus = "Menghubungkan...";
    try {
      await invoke<string>("connect_port", { portName: selectedPort });
      isConnected = true;
      connectionStatus = `Connected to ${selectedPort}`;
      serialBuffer = "";
      lastTelemetryAt = 0;
      isTelemetryStale = false;
      startedAt = performance.now();
      lastChartPointAt = 0;
      chartCounter = 0;
      tData.length = 0;
      actualData.length = 0;
      powerData.length = 0;
      chartNeedsRedraw = true;
      scheduleChartRedraw();
    } catch (error) {
      isConnected = false;
      const errorMsg = error instanceof Error ? error.message : String(error);
      connectionStatus = `Connect gagal: ${errorMsg}`;
      console.error("Connection error:", error);
    }
  }

  async function disconnectSerial() {
    try {
      await invoke("disconnect_port");
    } catch (error) {
      console.error("Disconnect error:", error);
    } finally {
      isConnected = false;
      connectionStatus = "Disconnected";
      serialBuffer = "";
      lastTelemetryAt = 0;
      isTelemetryStale = false;
      chartCounter = 0;
      tData.length = 0;
      actualData.length = 0;
      powerData.length = 0;
      startedAt = 0;
      lastChartPointAt = 0;
      chartNeedsRedraw = true;
      scheduleChartRedraw();
    }
  }

  async function toggleSerialConnection() {
    if (isConnectionBusy) return;

    isConnectionBusy = true;
    try {
      if (isConnected) {
        await disconnectSerial();
      } else {
        await connectSerial();
      }
    } finally {
      isConnectionBusy = false;
    }
  }

  onMount(async () => {
    const savedTheme = localStorage.getItem("t210-theme");
    applyTheme(savedTheme === "light" ? "light" : "dark");

    appVersion = await getVersion();

    await scanPorts();
    initPlot();

    staleTimer = setInterval(updateStaleState, 1000);

    unlistenSerialData = await listen<string>("serial-data", (event) => {
      onSerialData(event.payload);
    });

    unlistenSerialDisconnected = await listen<string>("serial-disconnected", (event) => {
      isConnected = false;
      connectionStatus = event.payload || "Disconnected";
      serialBuffer = "";
      lastTelemetryAt = 0;
      isTelemetryStale = false;
      chartCounter = 0;
      tData.length = 0;
      actualData.length = 0;
      powerData.length = 0;
      startedAt = 0;
      lastChartPointAt = 0;
      chartNeedsRedraw = true;
      scheduleChartRedraw();
    });
  });

  onDestroy(() => {
    if (unlistenSerialData) unlistenSerialData();
    if (unlistenSerialDisconnected) unlistenSerialDisconnected();
    if (staleTimer) clearInterval(staleTimer);
    if (resizeHandler) window.removeEventListener("resize", resizeHandler);
    resizeObserver?.disconnect();
    if (chartFrameId !== null) cancelAnimationFrame(chartFrameId);
    if (isConnected) void invoke("disconnect_port");
    plot?.destroy();
  });
</script>

<div class="page-shell" class:theme-light={theme === "light"}>
  <main class="dashboard">
    <section class="panel side-panel left-panel">
      <div class="panel-content">
        <div class="control-group">
          <label for="serial-port" class="field-label">Serial Port</label>
          <div class="port-row">
            <select id="serial-port" bind:value={selectedPort} class="custom-select" disabled={isConnected} on:blur>
              {#each ports as port}
                <option value={port.port_name} title={port.display_name}>{port.display_name}</option>
              {/each}
            </select>
            <button
              class="btn btn-scan btn-scan-inline"
              on:click={scanPorts}
              disabled={isConnected}
              aria-label="Scan Port"
              title="Scan Port"
            >
              <img class="scan-icon" src={refreshIcon} alt="" aria-hidden="true" />
            </button>
          </div>
        </div>

        <div class="status-box">
          <div class="field-label">Connection</div>
          <div class="status-value" class:color-yellow={isTelemetryStale}>{connectionDisplay}</div>
        </div>

        <button
          class="btn"
          class:btn-connect={!isConnected}
          class:btn-disconnect={isConnected}
          on:click={toggleSerialConnection}
          disabled={isConnectionBusy || (!isConnected && !selectedPort)}
        >
          {isConnectionBusy ? "Processing..." : isConnected ? "Disconnect" : "Connect"}
        </button>

        <div class="status-box">
          <div class="field-label">Solder Status</div>
          <div
            class="status-value"
            class:color-gray={statusInfo.color === "color-gray"}
            class:color-yellow={statusInfo.color === "color-yellow"}
            class:color-green={statusInfo.color === "color-green"}
            class:color-red={statusInfo.color === "color-red"}
            title={statusInfo.label}
          >
            {statusInfo.text}
          </div>
        </div>

        <div class="status-box">
          <div class="field-label">Buzzer</div>
          <div class="status-value color-green">{buzzerText}</div>
        </div>

        <div class="status-box">
          <div class="field-label">Standby State</div>
          <div class="status-value">{standbyText}</div>
        </div>

        <div class="status-box">
          <div class="field-label">Standby Timer</div>
          <div class="status-value">{standbyTimer === null ? "-- s" : `${formatNumber(standbyTimer, 0)} s`}</div>
        </div>

      </div>
    </section>

    <section class="main-content">
      <div class="top-display">
        <div class="gauge-box">
          <div class="field-label">Setpoint</div>
          <div class="value large-text">{formatNumber(setpoint, 0)}<span>°C</span></div>
        </div>
        <div class="gauge-box">
          <div class="field-label">Actual Temp</div>
          <div class="value large-text color-orange">{formatNumber(actualTemp, 0)}<span>°C</span></div>
        </div>
      </div>

      <div class="graph-container">
        <div class="field-label">Temperature / Power</div>
        <div class="chart-side-labels" aria-hidden="true">
          <span>Temp (°C)</span>
          <span>Power (%)</span>
        </div>
        <div id="uplot-target" bind:this={chartTarget}></div>
        <div class="chart-legend">
          <div class="legend-item">
            <div class="legend-dot actual-temp"></div>
            <span>Actual Temp</span>
          </div>
          <div class="legend-item">
            <div class="legend-dot power"></div>
            <span>Power</span>
          </div>
        </div>
      </div>
    </section>

  </main>

  <button class="theme-trigger" type="button" on:click={toggleTheme} aria-label="Toggle theme" title="Toggle theme">
    {theme === "dark" ? "Light" : "Dark"}
  </button>

  <button class="about-trigger" type="button" on:click={() => (showAbout = true)} aria-label="About ABToolkit" title="About">
    About
  </button>

  {#if showAbout}
    <div class="about-backdrop">
      <button class="about-backdrop-close" type="button" on:click={() => (showAbout = false)} aria-label="Close About panel"></button>
      <div class="about-modal" role="dialog" aria-modal="true" aria-labelledby="about-title">
        <div class="about-header">
          <div>
            <div class="field-label">About</div>
            <h2 id="about-title">ABToolkit</h2>
          </div>
          <button class="about-close" type="button" on:click={() => (showAbout = false)} aria-label="Close About panel">
            ×
          </button>
        </div>

        <p class="about-subtitle">T210 High End Monitoring Utility</p>

        <div class="about-grid">
          <div class="about-item">
            <span>Produk</span>
            <strong>T210 High End • App v{appVersion}</strong>
          </div>
          <div class="about-item">
            <span>Email</span>
            <strong>info@abtoolkit.id</strong>
          </div>
          <div class="about-item">
            <span>Website</span>
            <button class="about-link" type="button" on:click={openWebsite}>https://abtoolkit.id/</button>
          </div>
        </div>

        <div class="about-note">
          Peralatan servis elektronik lokal yang andal dan terjangkau untuk teknisi Indonesia.
        </div>

        <div class="about-footer">© 2026 ABToolkit / Produk Abimanyu Blitar. All rights reserved.</div>
      </div>
    </div>
  {/if}
</div>

<style>
  @font-face {
    font-family: "Inter";
    src: url("/fonts/Inter-SemiBold.woff2") format("woff2");
    font-weight: 600;
    font-style: normal;
    font-display: swap;
  }

  @font-face {
    font-family: "Inter";
    src: url("/fonts/Inter-Black.woff2") format("woff2");
    font-weight: 900;
    font-style: normal;
    font-display: swap;
  }

  :global(body) {
    --app-bg: linear-gradient(135deg, #0a0a0f 0%, #0f0f1a 100%);
    --text-main: #e2e8f0;
    --text-strong: #f8fafc;
    --text-muted: #94a3b8;
    --text-subtle: #64748b;
    --panel-bg: linear-gradient(135deg, #1a1a28 0%, #16161f 100%);
    --panel-border: #2a2a38;
    --panel-border-hover: #3a3a48;
    --gauge-bg: linear-gradient(135deg, #1a2332 0%, #162847 100%);
    --graph-bg: linear-gradient(135deg, #0f1419 0%, #0a0f18 100%);
    --graph-border: #1e3a5f;
    --control-bg: linear-gradient(135deg, #1a1a28 0%, #16161f 100%);
    --scan-btn-bg: linear-gradient(135deg, #1a1a28 0%, #16161f 100%);
    --scan-btn-bg-hover: linear-gradient(135deg, #222a38 0%, #1a222e 100%);
    --scan-btn-border: #2a3a50;
    --scan-btn-color: #60a5fa;
    --scan-icon-filter: none;
    --button-glass-bg: rgba(15, 23, 42, 0.72);
    --button-glass-bg-hover: rgba(30, 58, 138, 0.76);
    --floating-btn-color: #93c5fd;
    --floating-btn-color-hover: #dbeafe;
    --floating-btn-border: rgba(96, 165, 250, 0.35);
    --floating-btn-border-hover: #60a5fa;
    --floating-btn-shadow: 0 5px 14px rgba(0, 0, 0, 0.3);
    --modal-bg: linear-gradient(135deg, #111827 0%, #0f172a 100%);
    --about-item-bg: rgba(15, 23, 42, 0.68);
    --input-text: #ffffff;
    --select-bg-solid: #16161f;
    --select-disabled-bg: #16161f;
    --select-option-bg: #1a1a28;
    --select-option-color: #ffffff;
    --select-hover-border: #3a3a48;
    --chart-grid: #1f2937;
    --chart-axis: #94a3b8;
    --status-shadow: 0 4px 10px rgba(0, 0, 0, 0.3), inset 0 1px 2px rgba(255, 255, 255, 0.03);
    --status-shadow-hover: 0 6px 14px rgba(0, 0, 0, 0.4), inset 0 1px 2px rgba(255, 255, 255, 0.05);
    --shadow-panel: 0 8px 16px rgba(0, 0, 0, 0.4), inset 0 1px 2px rgba(255, 255, 255, 0.03);
    --shadow-panel-hover: 0 10px 20px rgba(0, 0, 0, 0.5), inset 0 1px 2px rgba(255, 255, 255, 0.05);
    margin: 0;
    padding: 0;
    background: var(--app-bg);
    color: var(--text-main);
    font-family: "Inter", sans-serif;
    font-weight: 600;
    overflow-x: hidden;
    overflow-y: hidden;
  }

  :global(body.theme-light) {
    --app-bg: linear-gradient(135deg, #eaf1ff 0%, #f8fafc 100%);
    --text-main: #1e293b;
    --text-strong: #0f172a;
    --text-muted: #475569;
    --text-subtle: #64748b;
    --panel-bg: linear-gradient(135deg, #ffffff 0%, #f1f5f9 100%);
    --panel-border: #cbd5e1;
    --panel-border-hover: #93c5fd;
    --gauge-bg: linear-gradient(135deg, #eff6ff 0%, #dbeafe 100%);
    --graph-bg: linear-gradient(135deg, #ffffff 0%, #eff6ff 100%);
    --graph-border: #bfdbfe;
    --control-bg: linear-gradient(135deg, #ffffff 0%, #f8fafc 100%);
    --scan-btn-bg: linear-gradient(135deg, #dbeafe 0%, #bfdbfe 100%);
    --scan-btn-bg-hover: linear-gradient(135deg, #bfdbfe 0%, #93c5fd 100%);
    --scan-btn-border: #3b82f6;
    --scan-btn-color: #1d4ed8;
    --scan-icon-filter: brightness(0) saturate(100%) invert(31%) sepia(91%) saturate(1475%) hue-rotate(210deg) brightness(92%) contrast(93%);
    --button-glass-bg: rgba(255, 255, 255, 0.82);
    --button-glass-bg-hover: rgba(219, 234, 254, 0.95);
    --floating-btn-color: #1d4ed8;
    --floating-btn-color-hover: #1e3a8a;
    --floating-btn-border: rgba(37, 99, 235, 0.45);
    --floating-btn-border-hover: #2563eb;
    --floating-btn-shadow: 0 5px 14px rgba(37, 99, 235, 0.16);
    --modal-bg: linear-gradient(135deg, #ffffff 0%, #eff6ff 100%);
    --about-item-bg: rgba(241, 245, 249, 0.88);
    --input-text: #0f172a;
    --select-bg-solid: #ffffff;
    --select-disabled-bg: #ffffff;
    --select-option-bg: #eff6ff;
    --select-option-color: #0f172a;
    --select-hover-border: #2563eb;
    --chart-grid: #dbeafe;
    --chart-axis: #475569;
    --status-shadow: 0 4px 12px rgba(37, 99, 235, 0.1), inset 0 1px 2px rgba(255, 255, 255, 0.75);
    --status-shadow-hover: 0 6px 16px rgba(37, 99, 235, 0.16), inset 0 1px 2px rgba(255, 255, 255, 0.85);
    --shadow-panel: 0 8px 18px rgba(37, 99, 235, 0.12), inset 0 1px 2px rgba(255, 255, 255, 0.75);
    --shadow-panel-hover: 0 10px 22px rgba(37, 99, 235, 0.18), inset 0 1px 2px rgba(255, 255, 255, 0.85);
  }

  :global(html),
  :global(body) {
    scrollbar-width: none;
    -ms-overflow-style: none;
  }

  :global(html::-webkit-scrollbar),
  :global(body::-webkit-scrollbar) {
    display: none;
  }

  :global(button),
  :global(input),
  :global(select),
  :global(textarea) {
    font-family: "Inter", sans-serif;
  }

  .page-shell {
    height: 100dvh;
    overflow: hidden;
  }

  .dashboard {
    --btn-h: 42px;
    --radius-sm: 8px;
    display: grid;
    grid-template-columns: clamp(200px, 22vw, 280px) minmax(0, 1fr);
    gap: 10px;
    padding: 10px;
    height: 100dvh;
    box-sizing: border-box;
    overflow: hidden;
  }

  .panel {
    background: var(--panel-bg);
    border-radius: 12px;
    padding: 14px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    border: 1.5px solid var(--panel-border);
    box-shadow: var(--shadow-panel);
    transition: border-color 0.3s ease, box-shadow 0.3s ease;
  }

  .panel:hover {
    border-color: var(--panel-border-hover);
    box-shadow: var(--shadow-panel-hover);
  }

  .side-panel {
    min-width: 0;
    overflow: hidden;
    position: relative;
  }

  .left-panel {
    overflow-y: auto;
    scrollbar-width: thin;
    scrollbar-color: #2563eb var(--panel-border);
  }

  .left-panel::-webkit-scrollbar {
    width: 6px;
  }

  .left-panel::-webkit-scrollbar-track {
    background: var(--panel-border);
  }

  .left-panel::-webkit-scrollbar-thumb {
    background: #2563eb;
    border-radius: 3px;
  }

  .left-panel::-webkit-scrollbar-thumb:hover {
    background: #3b82f6;
  }

  .left-panel,
  .main-content {
    scrollbar-width: none;
    -ms-overflow-style: none;
  }

  .left-panel::-webkit-scrollbar,
  .main-content::-webkit-scrollbar {
    display: none;
  }

  .panel-content {
    display: flex;
    flex-direction: column;
    gap: 14px;
    min-height: 0;
  }

  .main-content {
    min-width: 0;
    min-height: 0;
    overflow: hidden;
    height: 100%;
    position: relative;
    display: grid;
    grid-template-rows: minmax(80px, 140px) minmax(150px, 300px);
    gap: 8px;
  }

  .theme-trigger,
  .about-trigger {
    position: fixed;
    bottom: clamp(38px, 7vh, 54px);
    z-index: 20;
    min-height: clamp(30px, 5vw, 38px);
    padding: 0 clamp(10px, 2vw, 14px);
    border-radius: 9px;
    border: 1.5px solid var(--floating-btn-border);
    background: var(--button-glass-bg);
    color: var(--floating-btn-color);
    font-size: clamp(0.68rem, 1.25vw, 0.78rem);
    font-weight: 800;
    letter-spacing: 0.35px;
    line-height: 1;
    cursor: pointer;
    backdrop-filter: blur(10px);
    box-shadow: var(--floating-btn-shadow);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    transform: translateX(-50%);
    transition: transform 0.2s ease, border-color 0.2s ease, color 0.2s ease, background 0.2s ease;
  }

  .about-trigger {
    left: calc(10px + (clamp(200px, 22vw, 280px) / 2) + 36px);
  }

  .theme-trigger {
    left: calc(10px + (clamp(200px, 22vw, 280px) / 2) - 36px);
  }

  .theme-trigger:hover,
  .about-trigger:hover {
    transform: translateX(-50%) translateY(-1px);
    border-color: var(--floating-btn-border-hover);
    color: var(--floating-btn-color-hover);
    background: var(--button-glass-bg-hover);
  }

  .about-backdrop {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: grid;
    place-items: center;
    padding: clamp(12px, 3vw, 18px);
    background: rgba(2, 6, 23, 0.68);
    backdrop-filter: blur(8px);
  }

  .about-backdrop-close {
    position: absolute;
    inset: 0;
    border: 0;
    padding: 0;
    background: transparent;
    cursor: default;
  }

  .about-modal {
    position: relative;
    z-index: 1;
    width: min(420px, calc(100vw - 28px));
    max-height: calc(100dvh - 28px);
    overflow: auto;
    border-radius: clamp(14px, 3vw, 18px);
    border: 1.5px solid rgba(96, 165, 250, 0.35);
    background: var(--modal-bg);
    box-shadow: 0 24px 70px rgba(0, 0, 0, 0.6), inset 0 1px 2px rgba(255, 255, 255, 0.05);
    padding: clamp(14px, 3vw, 18px);
  }

  .about-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 14px;
  }

  .about-header h2 {
    margin: 0;
    color: var(--text-strong);
    font-size: clamp(1.2rem, 4vw, 1.5rem);
    line-height: 1.1;
    letter-spacing: -0.4px;
  }

  .about-close {
    width: clamp(30px, 6vw, 34px);
    height: clamp(30px, 6vw, 34px);
    border: 1.5px solid #334155;
    border-radius: 10px;
    background: var(--control-bg);
    color: var(--text-main);
    font-size: clamp(1.15rem, 4vw, 1.35rem);
    line-height: 1;
    cursor: pointer;
    transition: border-color 0.2s ease, color 0.2s ease, background 0.2s ease;
  }

  .about-close:hover {
    border-color: #60a5fa;
    color: var(--text-strong);
    background: var(--button-glass-bg-hover);
  }

  .about-subtitle {
    margin: 7px 0 13px;
    color: #93c5fd;
    font-size: clamp(0.78rem, 2vw, 0.9rem);
  }

  .about-grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: clamp(7px, 2vw, 10px);
    margin-bottom: 12px;
  }

  .about-item {
    padding: clamp(9px, 2.2vw, 12px);
    border-radius: 11px;
    border: 1px solid rgba(51, 65, 85, 0.85);
    background: var(--about-item-bg);
  }

  .about-item span {
    display: block;
    margin-bottom: 5px;
    color: var(--text-subtle);
    font-size: 0.68rem;
    text-transform: uppercase;
    letter-spacing: 0.8px;
  }

  .about-item strong {
    color: var(--text-main);
    font-size: clamp(0.74rem, 2vw, 0.86rem);
    line-height: 1.3;
  }

  .about-link {
    padding: 0;
    border: 0;
    background: transparent;
    color: #93c5fd;
    font-size: clamp(0.74rem, 2vw, 0.86rem);
    font-weight: 800;
    line-height: 1.3;
    text-align: left;
    text-decoration: underline;
    text-underline-offset: 3px;
    cursor: pointer;
  }

  .about-link:hover {
    color: #dbeafe;
  }

  .about-note {
    padding: clamp(10px, 2.4vw, 12px);
    border-radius: 11px;
    background: rgba(37, 99, 235, 0.1);
    border: 1px solid rgba(37, 99, 235, 0.25);
    color: var(--text-main);
    font-size: clamp(0.74rem, 2vw, 0.82rem);
    line-height: 1.45;
  }

  .about-footer {
    margin-top: 12px;
    color: var(--text-subtle);
    font-size: clamp(0.64rem, 1.8vw, 0.72rem);
    text-align: center;
  }

  @media (max-width: 520px) {
    .about-grid {
      grid-template-columns: 1fr;
    }
  }

  .field-label {
    font-size: 0.75rem;
    font-weight: 700;
    color: #60a5fa;
    text-transform: uppercase;
    letter-spacing: 1px;
    margin-bottom: 4px;
    display: block;
    opacity: 0.9;
    word-wrap: break-word;
    overflow-wrap: break-word;
  }

  .top-display {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 12px;
    margin-bottom: 0;
    padding: 4px 0;
    min-width: 0;
  }

  .gauge-box {
    background: var(--gauge-bg);
    padding: 20px 18px;
    border-radius: 12px;
    border: 2px solid #2563eb;
    box-shadow: 0 8px 16px rgba(37, 99, 235, 0.15), inset 0 1px 2px rgba(255, 255, 255, 0.05);
    transition: transform 0.3s ease, box-shadow 0.3s ease, border-color 0.3s ease;
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 8px;
    min-width: 0;
    overflow: hidden;
  }

  .gauge-box:hover {
    transform: translateY(-2px);
    border-color: #3b82f6;
    box-shadow: 0 12px 24px rgba(37, 99, 235, 0.25), inset 0 1px 2px rgba(255, 255, 255, 0.08);
  }

  .large-text {
    font-size: clamp(2.4rem, 5vw, 5.5rem);
    font-weight: 900;
    font-family: "Inter", sans-serif;
    letter-spacing: -0.5px;
    line-height: 1.1;
    min-width: 0;
    white-space: nowrap;
  }

  .large-text span {
    font-size: 0.55em;
    margin-left: 6px;
    flex-shrink: 0;
  }

  .color-orange {
    color: #fb923c;
    text-shadow: 0 0 20px rgba(251, 146, 60, 0.3);
  }

  .color-green {
    color: #4ade80;
    text-shadow: 0 0 15px rgba(74, 222, 128, 0.3);
  }

  .color-yellow {
    color: #f59e0b;
    font-weight: 600;
    text-shadow: 0 0 15px rgba(245, 158, 11, 0.3);
  }

  .color-gray {
    color: #9ca3af;
    opacity: 0.8;
  }

  .color-red {
    color: #ef4444;
    font-weight: 600;
    text-shadow: 0 0 15px rgba(239, 68, 68, 0.3);
    animation: pulse-error 0.6s ease-in-out infinite;
  }

  @keyframes pulse-error {
    0%, 100% {
      opacity: 1;
    }
    50% {
      opacity: 0.7;
    }
  }

  .graph-container {
    background: var(--graph-bg);
    border-radius: 12px;
    padding: 10px;
    min-width: 0;
    min-height: 0;
    max-height: 100%;
    overflow: hidden;
    border: 1.5px solid var(--graph-border);
    box-shadow: 0 4px 16px rgba(37, 99, 235, 0.1), inset 0 1px 2px rgba(96, 165, 250, 0.05);
    display: grid;
    grid-template-rows: auto auto 1fr auto;
    gap: 6px;
    transition: border-color 0.3s ease, box-shadow 0.3s ease;
  }

  .graph-container:hover {
    border-color: #2563eb;
    box-shadow: 0 6px 20px rgba(37, 99, 235, 0.15), inset 0 1px 2px rgba(96, 165, 250, 0.08);
  }

  .chart-side-labels {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-top: 2px;
    margin-bottom: 4px;
    color: #94a3b8;
    font-size: 0.74rem;
    font-weight: 600;
    letter-spacing: 0.2px;
  }

  .chart-legend {
    display: flex;
    justify-content: center;
    align-items: center;
    gap: 24px;
    padding: 8px 0;
    border-top: 1px solid rgba(37, 99, 235, 0.2);
    margin-top: 4px;
  }

  .legend-item {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-muted);
    font-size: 0.8rem;
    font-weight: 500;
    letter-spacing: 0.3px;
  }

  .legend-dot {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .legend-dot.actual-temp {
    background: #fb923c;
    box-shadow: 0 0 8px rgba(251, 146, 60, 0.5);
  }

  .legend-dot.power {
    background: #4ade80;
    box-shadow: 0 0 8px rgba(74, 222, 128, 0.5);
  }

  #uplot-target {
    width: 100%;
    height: 100%;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
    border: 1.5px solid var(--graph-border);
    border-radius: 8px;
    margin: 0;
    background: var(--graph-bg);
  }

  .custom-select {
    width: 100%;
    min-height: var(--btn-h);
    padding: 8px 10px;
    box-sizing: border-box;
    background: var(--select-bg-solid);
    background-color: var(--select-bg-solid);
    border: 1.5px solid var(--panel-border);
    color: var(--input-text);
    -webkit-text-fill-color: var(--input-text);
    border-radius: var(--radius-sm);
    font-size: 0.95rem;
    transition: all 0.2s ease;
    box-shadow: inset 0 2px 4px rgba(0, 0, 0, 0.3);
    appearance: auto;
  }

  .custom-select option {
    background: var(--select-option-bg);
    background-color: var(--select-option-bg);
    color: var(--select-option-color);
    -webkit-text-fill-color: var(--select-option-color);
    padding: 8px;
  }

  .custom-select option:checked {
    background: linear-gradient(#2563eb, #2563eb);
    color: white;
    -webkit-text-fill-color: white;
  }

  .custom-select:disabled {
    background: var(--select-disabled-bg);
    background-color: var(--select-disabled-bg);
    color: var(--input-text);
    -webkit-text-fill-color: var(--input-text);
    opacity: 1;
  }

  :global(body.theme-light) .custom-select,
  :global(body.theme-light) .custom-select:disabled {
    background: #ffffff;
    background-color: #ffffff;
    color: #0f172a;
    -webkit-text-fill-color: #0f172a;
    border-color: #94a3b8;
  }

  :global(body.theme-light) .custom-select option {
    background: #ffffff;
    background-color: #ffffff;
    color: #0f172a;
    -webkit-text-fill-color: #0f172a;
  }

  .custom-select:hover:not(:disabled) {
    border-color: var(--select-hover-border);
    box-shadow: inset 0 2px 4px rgba(0, 0, 0, 0.3), 0 2px 8px rgba(0, 0, 0, 0.2);
  }

  .custom-select:focus:not(:disabled) {
    border-color: #2563eb;
    box-shadow: inset 0 2px 4px rgba(0, 0, 0, 0.3), 0 0 12px rgba(37, 99, 235, 0.3);
  }

  .port-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 8px;
    align-items: stretch;
  }

  .btn {
    min-height: var(--btn-h);
    padding: 0 12px;
    border: none;
    border-radius: var(--radius-sm);
    font-weight: 600;
    font-size: 0.95rem;
    cursor: pointer;
    transition: all 0.2s ease;
    box-shadow: 0 2px 6px rgba(0, 0, 0, 0.2);
  }

  .btn:hover:not(:disabled) {
    transform: translateY(-1px);
    box-shadow: 0 4px 10px rgba(0, 0, 0, 0.3);
  }

  .btn:active:not(:disabled) {
    transform: translateY(0);
  }

  .btn:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .btn:focus-visible,
  .custom-select:focus-visible {
    outline: 2px solid #2563eb;
    outline-offset: 2px;
  }

  .custom-select:focus {
    border-color: #2563eb;
    box-shadow: inset 0 2px 4px rgba(0, 0, 0, 0.3), 0 0 12px rgba(37, 99, 235, 0.3);
  }

  .custom-select:not(:focus) {
    border-color: var(--panel-border);
    box-shadow: inset 0 2px 4px rgba(0, 0, 0, 0.3);
  }

  .btn-scan {
    background: var(--scan-btn-bg);
    color: var(--scan-btn-color);
    border: 1.5px solid var(--scan-btn-border);
    transition: all 0.2s ease;
  }

  .btn-scan:hover:not(:disabled) {
    background: var(--scan-btn-bg-hover);
    border-color: #2563eb;
    color: var(--scan-btn-color);
    box-shadow: 0 4px 10px rgba(37, 99, 235, 0.2);
  }

  .btn-scan-inline {
    width: var(--btn-h);
    min-width: var(--btn-h);
    padding: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }

  .scan-icon {
    width: 20px;
    height: 20px;
    opacity: 1;
    filter: var(--scan-icon-filter);
    display: block;
  }

  .btn-connect {
    background: linear-gradient(135deg, #1e3a8a 0%, #1e40af 100%);
    color: white;
    border: 1.5px solid #2563eb;
    transition: all 0.2s ease;
  }

  .btn-connect:hover:not(:disabled) {
    background: linear-gradient(135deg, #2563eb 0%, #3b82f6 100%);
    border-color: #60a5fa;
    box-shadow: 0 4px 12px rgba(37, 99, 235, 0.4);
  }

  .btn-disconnect {
    background: var(--control-bg);
    color: var(--text-muted);
    margin-top: 0;
    border: 1.5px solid var(--panel-border);
    transition: all 0.2s ease;
  }

  .btn-disconnect:hover:not(:disabled) {
    background: linear-gradient(135deg, #22222c 0%, #1a1a22 100%);
    border-color: #3a3a48;
    color: #cbd5e1;
    box-shadow: 0 4px 10px rgba(0, 0, 0, 0.3);
  }

  .status-box {
    background: var(--control-bg);
    padding: 12px 14px;
    border-radius: 10px;
    text-align: center;
    border: 1.5px solid var(--panel-border);
    box-shadow: var(--status-shadow);
    transition: all 0.2s ease;
  }

  .status-box:hover {
    border-color: var(--panel-border-hover);
    box-shadow: var(--status-shadow-hover);
  }

  .status-value {
    font-size: 1.05rem;
    font-weight: 700;
    margin-top: 4px;
    letter-spacing: 0.5px;
  }

  .left-panel .panel-content {
    height: 100%;
    justify-content: flex-start;
    padding-bottom: clamp(58px, 11vh, 76px);
    box-sizing: border-box;
  }

  .left-panel .btn {
    width: 100%;
  }

  /* Untuk ukuran sangat kecil (500px - 660px height) */
  @media (max-height: 660px) {
    .dashboard {
      gap: 8px;
      padding: 8px;
    }

    .panel {
      padding: 10px;
      gap: 8px;
    }

    .panel-content {
      gap: 10px;
    }

    .main-content {
      grid-template-rows: 80px 1fr;
      gap: 8px;
    }

    .gauge-box {
      padding: 10px;
    }

    .large-text {
      font-size: clamp(1.4rem, 3.5vw, 2.2rem);
    }

    .graph-container {
      padding: 8px;
      gap: 4px;
    }

    .chart-side-labels {
      margin: 0;
      font-size: 0.65rem;
    }

    #uplot-target {
      margin: 0;
    }

    .status-box {
      padding: 7px 9px;
    }

    .btn {
      min-height: 36px;
      font-size: 0.9rem;
    }

    .custom-select {
      min-height: 36px;
      font-size: 0.9rem;
      padding: 6px 8px;
    }

    .field-label {
      font-size: 0.7rem;
      margin-bottom: 2px;
    }

    .status-value {
      font-size: 0.88rem;
    }

    .left-panel .panel-content {
      gap: 7px;
      padding-bottom: 70px;
    }
  }

  /* Untuk ukuran medium (661px - 899px height) */
  @media (min-height: 661px) and (max-height: 899px) {
    .dashboard {
      gap: 9px;
      padding: 9px;
    }

    .panel {
      padding: 12px;
      gap: 10px;
    }

    .panel-content {
      gap: 12px;
    }

    .main-content {
      grid-template-rows: 120px 1fr;
      gap: 10px;
    }

    .gauge-box {
      padding: 12px;
    }

    .large-text {
      font-size: clamp(1.8rem, 4.2vw, 3.2rem);
    }

    .graph-container {
      padding: 10px;
      gap: 5px;
    }

    .btn {
      min-height: 38px;
    }

    .custom-select {
      min-height: 38px;
    }
  }

  @media (max-height: 760px) {
    .dashboard {
      --btn-h: 36px;
      gap: 8px;
      padding: 8px;
    }

    .panel {
      padding: 10px;
      gap: 8px;
    }

    .panel-content {
      gap: 10px;
    }

    .main-content {
      grid-template-rows: 90px 1fr;
      gap: 8px;
    }

    .gauge-box {
      padding: 10px;
    }

    .large-text {
      font-size: clamp(1.7rem, 4.1vw, 3rem);
    }

    .graph-container {
      padding: 8px;
      gap: 4px;
    }

    .chart-side-labels {
      margin: 0;
      font-size: 0.68rem;
    }

    #uplot-target {
      margin: 0;
    }

    .status-box {
      padding: 8px 10px;
    }

    .left-panel .panel-content {
      padding-bottom: 72px;
    }
  }

  @media (min-height: 900px) {
    .dashboard {
      gap: 12px;
      padding: 12px;
    }

    .panel {
      padding: 16px;
      gap: 14px;
    }

    .panel-content {
      gap: 16px;
    }

    .main-content {
      grid-template-rows: 200px 1fr;
      gap: 14px;
    }

    .gauge-box {
      padding: 18px;
    }

    .large-text {
      font-size: clamp(4rem, 7vw, 8rem);
    }

    .graph-container {
      padding: 14px;
      gap: 8px;
    }
  }

  /* Saat windowed mode mendekati minimum, gauge tetap 2 kolom agar tinggi panel tidak membesar.
     Font dan padding diperkecil supaya Setpoint / Actual Temp tetap terlihat penuh. */
  @media (max-width: 760px) {
    .main-content {
      grid-template-rows: auto minmax(150px, 1fr);
    }

    .top-display {
      gap: 8px;
      padding: 0;
    }

    .gauge-box {
      padding: 10px 12px;
      gap: 4px;
    }

    .large-text {
      font-size: clamp(1.35rem, 6vw, 2.6rem);
      line-height: 1;
      letter-spacing: -0.8px;
    }

    .large-text span {
      margin-left: 4px;
    }
  }

  @media (max-width: 760px) and (min-height: 900px) {
    .main-content {
      grid-template-rows: auto minmax(150px, 1fr);
      gap: 10px;
    }

    .large-text {
      font-size: clamp(1.35rem, 5.5vw, 2.4rem);
    }
  }

  /* Hanya stack ke 1 kolom kalau lebar benar-benar sangat kecil. */
  @media (max-width: 480px) {
    .top-display {
      grid-template-columns: 1fr;
    }

    .large-text {
      font-size: clamp(1.6rem, 10vw, 2.8rem);
    }
  }

</style>
