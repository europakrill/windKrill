/**
 * windKrill application shell (M3): tabs, terminal panes, theme switching.
 *
 * M3 ships one pane per tab (no splits yet — docking lands in M3.5).
 */

import "./style.css";
import { applyTheme, THEMES } from "./theme";
import { bridge, type ScreenEnvelope } from "./session";
import { CanvasTerminalView } from "./canvas";

interface Tab {
  id: number;
  title: string;
  view: CanvasTerminalView;
  lastEnvelope: ScreenEnvelope | null;
}

const tabs: Tab[] = [];
let activeTab: Tab | null = null;
let themeId = "wind-dark";

const app = document.querySelector<HTMLDivElement>("#app")!;
app.innerHTML = `
  <div class="kr-titlebar">
    <span class="kr-logo">windKrill</span>
    <div class="kr-tabs" id="kr-tabs"></div>
    <button id="kr-new-tab" class="kr-btn" title="New tab">+</button>
    <select id="kr-theme" class="kr-select" title="Theme"></select>
  </div>
  <div class="kr-pane-host" id="kr-panes"></div>
  <div class="kr-statusbar">
    <span id="kr-status-text">ready</span>
    <span id="kr-backend-badge"></span>
  </div>
`;

const tabBar = document.querySelector<HTMLDivElement>("#kr-tabs")!;
const paneHost = document.querySelector<HTMLDivElement>("#kr-panes")!;
const statusText = document.querySelector<HTMLSpanElement>("#kr-status-text")!;
const backendBadge =
  document.querySelector<HTMLSpanElement>("#kr-backend-badge")!;

// Theme selector.
const themeSelect = document.querySelector<HTMLSelectElement>("#kr-theme")!;
for (const theme of Object.values(THEMES)) {
  const option = document.createElement("option");
  option.value = theme.id;
  option.textContent = theme.name;
  themeSelect.appendChild(option);
}
themeSelect.addEventListener("change", () => {
  themeId = themeSelect.value;
  applyTheme(THEMES[themeId]!);
});

applyTheme(THEMES[themeId]!);
backendBadge.textContent = bridge.isTauri()
  ? "engine: krill-session"
  : "engine: echo-sim (browser dev)";

function setStatus(text: string): void {
  statusText.textContent = text;
}

async function renderActive(): Promise<void> {
  if (!activeTab) return;
  try {
    const envelope = await bridge.screen(activeTab.id);
    activeTab.lastEnvelope = envelope;
    activeTab.view.render(envelope);
    setStatus(`tab #${activeTab.id} · ${envelope.cols}x${envelope.rows} · ${envelope.shell}`);
  } catch (error) {
    setStatus(`error: ${String(error)}`);
  }
}

function activate(tab: Tab): void {
  activeTab = tab;
  for (const t of tabs) {
    t.view.root.classList.toggle("hidden", t !== tab);
  }
  for (const el of Array.from(tabBar.children)) {
    el.classList.toggle("active", Number((el as HTMLElement).dataset.tab) === tab.id);
  }
  void renderActive();
}

function createTabEl(tab: Tab): HTMLElement {
  const el = document.createElement("div");
  el.className = "kr-tab";
  el.dataset.tab = String(tab.id);
  const label = document.createElement("span");
  label.textContent = tab.title;
  const closeBtn = document.createElement("button");
  closeBtn.className = "kr-tab-close";
  closeBtn.textContent = "×";
  closeBtn.addEventListener("click", (event) => {
    event.stopPropagation();
    void closeTab(tab);
  });
  el.append(label, closeBtn);
  el.addEventListener("click", () => activate(tab));
  return el;
}

async function newTab(): Promise<void> {
  const info = await bridge.createSession(80, 24);
  const container = document.createElement("div");
  container.className = "kr-pane";
  paneHost.appendChild(container);

  const tab: Tab = {
    id: info.id,
    title: `${info.shell} #${info.id}`,
    view: new CanvasTerminalView(container),
    lastEnvelope: null,
  };
  tab.view.setPalette(THEMES[themeId]!.colors.palette);
  tabs.push(tab);
  tabBar.appendChild(createTabEl(tab));
  activate(tab);
  // Focus keyboard input into the new tab.
  container.tabIndex = 0;
  container.focus();
}

async function closeTab(tab: Tab): Promise<void> {
  await bridge.closeSession(tab.id);
  const index = tabs.indexOf(tab);
  if (index >= 0) tabs.splice(index, 1);
  tab.view.root.remove();
  for (const el of Array.from(tabBar.children)) {
    if (Number((el as HTMLElement).dataset.tab) === tab.id) el.remove();
  }
  if (activeTab === tab) {
    activeTab = tabs[Math.max(0, index - 1)] ?? null;
    if (activeTab) activate(activeTab);
  }
  setStatus(`closed tab #${tab.id}`);
}

document.querySelector<HTMLButtonElement>("#kr-new-tab")!.addEventListener("click", () => {
  void newTab();
});

// Keyboard routing: printable keys + control sequences go to the session.
document.addEventListener("keydown", (event) => {
  if (!activeTab || (event.target as HTMLElement)?.tagName === "SELECT") return;
  let bytes: Uint8Array | null = null;
  if (event.key === "Enter") bytes = Uint8Array.of(13);
  else if (event.key === "Backspace") bytes = Uint8Array.of(127);
  else if (event.key === "Tab") bytes = Uint8Array.of(9);
  else if (event.key === "Escape") bytes = Uint8Array.of(27);
  else if (event.ctrlKey && event.key.length === 1) {
    const code = event.key.toUpperCase().charCodeAt(0);
    if (code >= 64 && code <= 95) bytes = Uint8Array.of(code - 64);
  } else if (!event.ctrlKey && !event.altKey && !event.metaKey && event.key.length === 1) {
    bytes = new TextEncoder().encode(event.key);
  }
  if (bytes) {
    event.preventDefault();
    void bridge.sendInput(activeTab.id, bytes).then(renderActive);
  }
});

// Resize handling: recompute grid from pane size and notify the PTY.
new ResizeObserver(() => {
  const tab = activeTab;
  if (!tab) return;
  tab.view.measure();
}).observe(paneHost);

// Polling loop: refresh the active tab's screen. Push events arrive in M4.
setInterval(() => {
  if (document.hasFocus() || true) void renderActive();
}, 120);

void newTab();
