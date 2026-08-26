/**
 * windKrill theme engine (M3).
 *
 * Themes are plain objects; the active theme is applied by setting CSS
 * custom properties on the document root. Adding a theme = adding an entry
 * to THEMES, no renderer changes needed.
 */

export interface Theme {
  id: string;
  name: string;
  colors: {
    bg: string;
    fg: string;
    cursor: string;
    selection: string;
    tabActiveBg: string;
    tabActiveFg: string;
    tabInactiveBg: string;
    tabInactiveFg: string;
    border: string;
    /** xterm 256-color palette (indices 0-15 required, 16-255 optional). */
    palette: string[];
  };
}

const basePalette = [
  "#000000", "#cd3131", "#0dbc79", "#e5e510",
  "#2472c8", "#bc3fbc", "#11a8cd", "#e5e5e5",
  "#666666", "#f14c4c", "#23d18b", "#f5f543",
  "#3b8eea", "#d670d6", "#29b8db", "#ffffff",
];

function full256(partial: string[]): string[] {
  const out = partial.slice();
  // 16..231: 6x6x6 color cube
  const steps = [0, 95, 135, 175, 215, 255];
  for (let r = 0; r < 6; r++)
    for (let g = 0; g < 6; g++)
      for (let b = 0; b < 6; b++)
        out.push(`rgb(${steps[r]},${steps[g]},${steps[b]})`);
  // 232..255: grayscale ramp
  for (let i = 0; i < 24; i++) {
    const v = 8 + i * 10;
    out.push(`rgb(${v},${v},${v})`);
  }
  return out;
}

export const THEMES: Record<string, Theme> = {
  "wind-dark": {
    id: "wind-dark",
    name: "Wind Dark",
    colors: {
      bg: "#1e1e1e",
      fg: "#cccccc",
      cursor: "#aeafad",
      selection: "#264f78",
      tabActiveBg: "#2d2d2d",
      tabActiveFg: "#ffffff",
      tabInactiveBg: "#181818",
      tabInactiveFg: "#9d9d9d",
      border: "#333333",
      palette: full256(basePalette),
    },
  },
  "wind-light": {
    id: "wind-light",
    name: "Wind Light",
    colors: {
      bg: "#ffffff",
      fg: "#1e1e1e",
      cursor: "#333333",
      selection: "#add6ff",
      tabActiveBg: "#f0f0f0",
      tabActiveFg: "#000000",
      tabInactiveBg: "#e4e4e4",
      tabInactiveFg: "#5a5a5a",
      border: "#c8c8c8",
      palette: full256([
        "#000000", "#c12437", "#1a7f37", "#7d6608",
        "#0b5cad", "#a626a4", "#0f7f8c", "#444444",
        "#666666", "#d24545", "#2da44e", "#9a8412",
        "#2188ff", "#c257c2", "#1b9aa8", "#eeeeee",
      ]),
    },
  },
};

export function applyTheme(theme: Theme): void {
  const root = document.documentElement;
  const c = theme.colors;
  root.style.setProperty("--kr-bg", c.bg);
  root.style.setProperty("--kr-fg", c.fg);
  root.style.setProperty("--kr-cursor", c.cursor);
  root.style.setProperty("--kr-selection", c.selection);
  root.style.setProperty("--kr-tab-active-bg", c.tabActiveBg);
  root.style.setProperty("--kr-tab-active-fg", c.tabActiveFg);
  root.style.setProperty("--kr-tab-inactive-bg", c.tabInactiveBg);
  root.style.setProperty("--kr-tab-inactive-fg", c.tabInactiveFg);
  root.style.setProperty("--kr-border", c.border);
  // Expose the first 16 ANSI entries for CSS; the renderer reads the full
  // palette from JS directly.
  for (let i = 0; i < 16; i++) {
    root.style.setProperty(`--kr-ansi-${i}`, c.palette[i] ?? "#888888");
  }
  root.dataset.theme = theme.id;
}
