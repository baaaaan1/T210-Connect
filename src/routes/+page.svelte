<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onDestroy, onMount } from "svelte";
  import uPlot from "uplot";
  import "uplot/dist/uPlot.min.css";
  import refreshIcon from "$lib/icons/solar--refresh-bold.svg";

  type Telemetry = {
  actualTemp: number | null;
  setpoint: number | null;
  power: number | null;
  status: number | null;
  buzzer: number | null;
  standbyState: number | null;
  standbyTimer: number | null;
};

  let ports: string[] = [];
  let selectedPort = "";
  let isConnected = false;
  let isConnectionBusy = false;
  let connectionStatus = "Disconnected";

  let actualTemp: number | null = null;
  let setpoint: number | null = null;
  let power: number | null = null;
  let status: number | null = null;
  let buzzer: number | null = null;
  let standbyState: number | null = null;
  let standbyTimer: number | null = null;
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

  function toNumOrNull(value: string | undefined): number | null {
    if (value === undefined) return null;
    const n = Number(value);
    return Number.isFinite(n) ? n : null;
  }

  function formatNumber(value: number | null, digits = 0): string {
    return value === null ? "--" : value.toFixed(digits);
  }

  function getStatusInfo(statusCode: number) {
    return (
      STATUS_CONDITIONS[statusCode] || {
        text: "Unknown",
        label: "Status tidak dikenali",
        color: "color-gray"
      }
    );
  }

  $: statusInfo = getStatusInfo(status ?? -1);
  $: buzzerText = buzzer === null ? "--" : buzzer === 1 ? "ON" : "OFF";
  $: standbyText = standbyState === null ? "--" : standbyState === 1 ? "Active" : "Inactive";

  let unlistenSerialData: (() => void) | null = null;
  let unlistenSerialDisconnected: (() => void) | null = null;

  let chartTarget: HTMLDivElement | null = null;
  let plot: uPlot | null = null;
  let resizeHandler: (() => void) | null = null;
  let resizeObserver: ResizeObserver | null = null;
  const maxPoints = 600;
  const windowSec = 60;
  const tData: number[] = [];
  const actualData: number[] = [];
  const powerData: number[] = [];
  let startedAt = 0;
  const temperatureTicks = [0, 100, 200, 300, 400, 500];
  const powerTicks = [0, 20, 40, 60, 80, 100];

  function syncPlotSize() {
    if (!plot || !chartTarget) return;
    plot.setSize({
      width: Math.max(chartTarget.clientWidth, 270),
      height: Math.max(chartTarget.clientHeight, 150)
    });
  }

  function parseTelemetry(payload: string): Telemetry | null {
  const parts = payload.trim().split(",");
  if (parts.length === 0 || !parts[0]) return null;

  return {
    actualTemp: toNumOrNull(parts[0]),
    setpoint: toNumOrNull(parts[1]),
    power: toNumOrNull(parts[2]),
    status: toNumOrNull(parts[3]),
    buzzer: toNumOrNull(parts[4]),
    standbyState: toNumOrNull(parts[5]),
    standbyTimer: toNumOrNull(parts[6])
  };
}

  function pushChartPoint(sample: { actualTemp: number; power: number }) {
    if (!chartReady) {
      chartQueue.push(sample);
      return;
    }

    if (!plot) return;

    const nowSec = (performance.now() - startedAt) / 1000;
    tData.push(nowSec);
    actualData.push(sample.actualTemp);
    powerData.push(sample.power);

    if (tData.length > maxPoints) {
      tData.shift();
      actualData.shift();
      powerData.shift();
    }

    plot.setData([tData, actualData, powerData]);
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
            range: (_u, _min, max) => [Math.max(0, max - windowSec), Math.max(max, windowSec)]
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
          { stroke: "#64748b", grid: { stroke: "#1f2937" }, values: (_u, vals) => vals.map((v) => `${v.toFixed(0)}s`) },
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
    ports = await invoke<string[]>("get_ports");
    if (ports.length > 0 && !selectedPort) {
      selectedPort = ports[0];
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
      startedAt = performance.now();
      tData.length = 0;
      actualData.length = 0;
      powerData.length = 0;
      plot?.setData([tData, actualData, powerData]);
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
    await scanPorts();
    initPlot();

    unlistenSerialData = await listen<string>("serial-data", (event) => {
      const telemetry = parseTelemetry(event.payload);
      if (!telemetry) return;

      actualTemp = telemetry.actualTemp;
      setpoint = telemetry.setpoint;
      power = telemetry.power;
      status = telemetry.status;
      buzzer = telemetry.buzzer;
      standbyState = telemetry.standbyState;
      standbyTimer = telemetry.standbyTimer;

      if (telemetry.actualTemp !== null && telemetry.power !== null) {
        pushChartPoint({ actualTemp: telemetry.actualTemp, power: telemetry.power });
      }
    });

    unlistenSerialDisconnected = await listen<string>("serial-disconnected", (event) => {
      isConnected = false;
      connectionStatus = event.payload || "Disconnected";
    });
  });

  onDestroy(() => {
    if (unlistenSerialData) unlistenSerialData();
    if (unlistenSerialDisconnected) unlistenSerialDisconnected();
    if (resizeHandler) window.removeEventListener("resize", resizeHandler);
    resizeObserver?.disconnect();
    if (isConnected) void invoke("disconnect_port");
    plot?.destroy();
  });
</script>

<div class="page-shell">
  <main class="dashboard">
    <section class="panel side-panel left-panel">
      <div class="panel-content">
        <div class="control-group">
          <label for="serial-port" class="field-label">Serial Port</label>
          <div class="port-row">
            <select id="serial-port" bind:value={selectedPort} class="custom-select" disabled={isConnected} on:blur>
              {#each ports as port}
                <option value={port}>{port}</option>
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
          <div class="status-value">{connectionStatus}</div>
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
    margin: 0;
    padding: 0;
    background: linear-gradient(135deg, #0a0a0f 0%, #0f0f1a 100%);
    color: #e2e8f0;
    font-family: "Inter", sans-serif;
    font-weight: 600;
    overflow-x: hidden;
    overflow-y: hidden;
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
    background: linear-gradient(135deg, #1a1a28 0%, #16161f 100%);
    border-radius: 12px;
    padding: 14px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    border: 1.5px solid #2a2a38;
    box-shadow: 0 8px 16px rgba(0, 0, 0, 0.4), inset 0 1px 2px rgba(255, 255, 255, 0.03);
    transition: border-color 0.3s ease, box-shadow 0.3s ease;
  }

  .panel:hover {
    border-color: #3a3a48;
    box-shadow: 0 10px 20px rgba(0, 0, 0, 0.5), inset 0 1px 2px rgba(255, 255, 255, 0.05);
  }

  .side-panel {
    min-width: 0;
    overflow: hidden;
    position: relative;
  }

  .left-panel {
    overflow-y: auto;
    scrollbar-width: thin;
    scrollbar-color: #2563eb #1a1a28;
  }

  .left-panel::-webkit-scrollbar {
    width: 6px;
  }

  .left-panel::-webkit-scrollbar-track {
    background: #1a1a28;
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
    display: grid;
    grid-template-rows: minmax(80px, 140px) minmax(150px, 300px);
    gap: 8px;
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
    grid-template-columns: 1fr 1fr;
    gap: 12px;
    margin-bottom: 0;
    padding: 4px 0;
    min-width: 0;
  }

  .gauge-box {
    background: linear-gradient(135deg, #1a2332 0%, #162847 100%);
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
    word-break: break-word;
    overflow-wrap: break-word;
  }

  .large-text span {
    font-size: 0.55em;
    margin-left: 6px;
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
    background: linear-gradient(135deg, #0f1419 0%, #0a0f18 100%);
    border-radius: 12px;
    padding: 10px;
    min-width: 0;
    min-height: 0;
    max-height: 100%;
    overflow: hidden;
    border: 1.5px solid #1e3a5f;
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
    color: #94a3b8;
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
    border: 1.5px solid #1e3a5f;
    border-radius: 8px;
    margin: 0;
    background: linear-gradient(135deg, #0a0f18 0%, #0f1419 100%);
  }

  .custom-select {
    width: 100%;
    min-height: var(--btn-h);
    padding: 8px 10px;
    box-sizing: border-box;
    background: linear-gradient(135deg, #1a1a28 0%, #16161f 100%);
    border: 1.5px solid #2a2a38;
    color: white;
    border-radius: var(--radius-sm);
    font-size: 0.95rem;
    transition: all 0.2s ease;
    box-shadow: inset 0 2px 4px rgba(0, 0, 0, 0.3);
  }

  .custom-select option {
    background: #1a1a28;
    color: white;
    padding: 8px;
  }

  .custom-select option:checked {
    background: linear-gradient(#2563eb, #2563eb);
    color: white;
  }

  .custom-select:hover:not(:disabled) {
    border-color: #3a3a48;
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
    border-color: #2a2a38;
    box-shadow: inset 0 2px 4px rgba(0, 0, 0, 0.3);
  }

  .btn-scan {
    background: linear-gradient(135deg, #1a1a28 0%, #16161f 100%);
    color: #60a5fa;
    border: 1.5px solid #2a3a50;
    transition: all 0.2s ease;
  }

  .btn-scan:hover:not(:disabled) {
    background: linear-gradient(135deg, #222a38 0%, #1a222e 100%);
    border-color: #2563eb;
    color: #93c5fd;
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
    opacity: 0.95;
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
    background: linear-gradient(135deg, #1a1a28 0%, #16161f 100%);
    color: #94a3b8;
    margin-top: 0;
    border: 1.5px solid #2a2a38;
    transition: all 0.2s ease;
  }

  .btn-disconnect:hover:not(:disabled) {
    background: linear-gradient(135deg, #22222c 0%, #1a1a22 100%);
    border-color: #3a3a48;
    color: #cbd5e1;
    box-shadow: 0 4px 10px rgba(0, 0, 0, 0.3);
  }

  .status-box {
    background: linear-gradient(135deg, #1a1a28 0%, #16161f 100%);
    padding: 12px 14px;
    border-radius: 10px;
    text-align: center;
    border: 1.5px solid #2a2a38;
    box-shadow: 0 4px 10px rgba(0, 0, 0, 0.3), inset 0 1px 2px rgba(255, 255, 255, 0.03);
    transition: all 0.2s ease;
  }

  .status-box:hover {
    border-color: #3a3a48;
    box-shadow: 0 6px 14px rgba(0, 0, 0, 0.4), inset 0 1px 2px rgba(255, 255, 255, 0.05);
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
      padding: 8px 10px;
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
      font-size: 0.95rem;
    }

    .left-panel .panel-content {
      gap: 8px;
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

</style>
